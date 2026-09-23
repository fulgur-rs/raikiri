use std::collections::HashMap;
use std::sync::Arc;

use crate::PseudoElem;
use crate::computed::{
    ComputedValues, CustomPropertyEnvironment, RunningTemplate, empty_custom_properties,
};
use crate::property::{
    BorderRadius, FontWeightValue, GridAutoFlowValue, GridLineValue, GridTemplateAreasValue,
    Length, LengthOrAuto, LengthOrNormal, PositionValue, PropertyValue, RelativeFontSize, Sides,
    WritingMode, initial_grid_auto_track_list, resolve_text_align_internal_center,
    resolve_text_align_match_parent,
};
use crate::resolve::{
    ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ResolveContext,
    used_line_height_length,
};
use crate::rule::{
    expand_background, expand_border, expand_border_color, expand_border_style,
    expand_border_width, expand_flex, expand_flex_flow, expand_font, expand_gap,
    expand_grid_column, expand_grid_row, expand_margin, expand_margin_block, expand_margin_inline,
    expand_outline, expand_overflow, expand_padding, expand_padding_block, expand_padding_inline,
    expand_place_content, expand_place_items, expand_place_self, expand_text_decoration,
};
use crate::ruletree::Origin;
use crate::specified::SpecifiedValues;
use crate::style_dom::{StyleDom, StyleNode, StyleNodeId, StyleNodeKind};

use super::collect::{CascadedArena, CascadedDecl, RankedDecl, pick_winners};
use super::custom_property::{resolve_custom_properties, resolve_deferred_value};

type InheritanceStackEntry = (
    StyleNodeId,
    ComputedValues,
    Option<ResolveContext>,
    Arc<CustomPropertyEnvironment>,
);

/// Top-down inheritance walk。子 node は親の computed value を必要とするため
/// (再帰の call stack で暗黙に運んでいた context)、iterative 化には各 stack
/// entry に `(StyleNodeId, 親の computed value, rem context)` を明示的に持たせる —
/// Approach A。clone は各 entry ごとに発生するが現時点の
/// scope では許容 (hot path 化した場合は将来 `Arc<ComputedValues>` で削減を検討)。
///
/// # `rem` context の threading (設計文書 §6.3)
///
/// stack entry 第 3 要素の `Option<ResolveContext>` は「この node より上に
/// **element 祖先が居るか**」を表す:
///
/// - `None` — element 祖先が無い。すなわちこの node が element なら **root
///   element** であり、[`SpecifiedValues::finalize_as_root`] を通す。同関数の
///   doc が CSS Values 4 §6.1.1
///   (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) の
///   parent-metrics 条項を verbatim で引き、`font-size` (initial 16px 基準) と
///   box property (自 font-size 基準) で `rem` の基準が違う理由を説明する。
///   **`html { font-size: 2rem }` が自己参照になる誤実装 (tree 全体に単一
///   context を配る) を塞ぐのはここ。**
/// - `Some(ctx)` — element 祖先が居る。その最上位 element (= root element) の
///   computed font-size が `ctx.root_font_size`、`lh` の値 (`rlh` の参照値、
///   `used_line_height_length` で絶対長化したもの。`normal` で解決不能なら
///   `None`) が `ctx.root_line_height`。
///
/// [`StyleDom::root_id`] は Document node であって root element ではない
/// ([`crate::style_dom`] の Contract 節) ため、Document / Comment / Text の
/// ような非 element node は `None` をそのまま子へ渡す。
///
/// well-formed な HTML document の root element は 1 つだが、`StyleDom` は
/// それを強制しない。Document 直下に element が複数ある合成 DOM では**各々が
/// root element として扱われる** (自分の subtree の `rem` 基準になる) —
/// 「親 element を持たない element は initial values を参照する」という §6.1.1
/// の規則を素直に適用した結果であり、意図した挙動である。
///
/// `pub(crate)` は他 module の doc からの intra-doc link のため — private 化で gate が red (規約 3)。
#[allow(clippy::too_many_arguments)] // the traversal writes several independent cascade outputs
pub(crate) fn resolve_inheritance<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    cascaded: &CascadedArena,
    out: &mut Vec<ComputedValues>,
    non_ua_margin_sides: &mut Vec<Sides<bool>>,
    authored_writing_modes: &mut Vec<Option<WritingMode>>,
    page_values: &mut [crate::property::PageValue],
    pseudo_out: &mut HashMap<(StyleNodeId, PseudoElem), ComputedValues>,
) {
    let mut stack: Vec<InheritanceStackEntry> =
        vec![(id, parent_computed.clone(), None, empty_custom_properties())];
    // `apply_winners` の scratch buffer。walk loop の**外**で確保して全 node で
    // 使い回す — per-node の `HashMap` 2 個が
    // n=1000 node で 3,667 allocs / 3.0 MB = cascade 全 heap traffic の 56.7%
    // を占めていた。buffer は最初の数 node で最大 `PropertyKey` index まで
    // 育ち、以降は 0 alloc。fill と drain は `apply_winners` に閉じており、
    // walk loop 側は「使い回す入れ物を貸す」以上の責務を持たない。
    let mut winners: Vec<Option<RankedDecl>> = Vec::new();
    while let Some((id, parent_computed, root_ctx, parent_custom_properties)) = stack.pop() {
        // is_in_document()==false
        // の node は subtree ごと早期 continue する。
        //
        // 以前は resize + write + children push を unconditional に行い computed
        // 長を node_count() に揃えていた。今 `cascade()` が
        // `dom.node_count()` で `computed` を pre-allocate + initial() で埋める
        // ように変わったため、visited しないままの slot は自然に initial()
        // として残る。これにより:
        //   - detached / template descendants は inherit_from(parent) の
        //     継承値ではなく initial() となる (`<template style="color:red">`
        //     配下は red を継承しない)
        //   - template subtree の walk が省ける (パフォーマンス改善)
        //
        // 未知 NodeId (dom.node が None) の場合も skip: initial() のままにする
        // 方が defensive (旧コードは inherit_from してから書いていた)。
        let Some(node) = dom.node(id).filter(|n| n.is_in_document()) else {
            continue;
        };
        let is_element = node.kind() == StyleNodeKind::Element;

        let local_custom_properties = cascaded
            .custom_candidates(id)
            .map(|candidates| resolve_custom_properties(&parent_custom_properties, candidates));
        let custom_properties = local_custom_properties
            .clone()
            .unwrap_or_else(|| parent_custom_properties.clone());
        let local_custom_properties =
            local_custom_properties.unwrap_or_else(empty_custom_properties);

        // phase 1: 親からの inheritance walk 開始値 (inherited のみ親の computed
        // からコピー、非継承は initial) に自 node の cascaded winner を適用する。
        // 適用対象は staging 表現なので winner の適用順に依存しない。
        let mut specified = SpecifiedValues::inherit_from(&parent_computed);
        let mut node_non_ua_margin = Sides::all(false);
        if authored_writing_modes.len() <= id.0 as usize {
            authored_writing_modes.resize(id.0 as usize + 1, None);
        }
        if let Some(candidates) = cascaded.candidates(id) {
            apply_winners(
                candidates,
                &mut winners,
                &mut specified,
                &parent_computed,
                &custom_properties,
                Some(&mut page_values[id.0 as usize]),
                Some(&mut node_non_ua_margin),
                Some(&mut authored_writing_modes[id.0 as usize]),
            );
        }

        // phase 2 + phase 3: 絶対化。root element (element 祖先なし) は `rem` の
        // 基準が phase 2 / phase 3 で異なるため専用 entry point を通す
        // (`SpecifiedValues::finalize_as_root` の doc に spec verbatim)。
        let mut computed = match &root_ctx {
            Some(ctx) => specified.finalize(&parent_computed, ctx),
            None => {
                // `finalize_as_root` は phase 2 の基準を initial value に固定する
                // (§6.1.1 の "if the element has no parent")。それが正しいのは
                // **`root_ctx == None` ならこの node に element 親が居ない**からで
                // あり、その caller-side invariant を check しておく:
                //
                // - `cascade()` は必ず `dom.root_id()` (= Document node) から
                //   walk を開始し、そこに `ComputedValues::initial()` を渡す。
                // - `collect_cascaded` は Element にしか winner を作らないので
                //   Document node の computed は initial のまま。
                // - `root_ctx` が `None` のままなのは Document 自身とその直接の子
                //   だけ (element を 1 つ通れば `Some` になる)。
                //
                // したがって subtree の途中から `resolve_inheritance` を呼ぶ
                // entry point (incremental restyle 等) を将来足すなら、
                // `root_ctx` を呼び出し側から供給しなければならない。この assert が
                // その見落としを debug build で捕まえる。
                debug_assert_eq!(
                    parent_computed.font_size,
                    ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
                    "root_ctx == None は element 親が居ないことを意味するので、\
                     親の computed font-size は initial でなければならない \
                     (subtree の途中から walk を開始していないか?)"
                );
                specified.finalize_as_root()
            }
        };
        computed.custom_properties = custom_properties.clone();
        computed.local_custom_properties = local_custom_properties;

        // 子へ渡す rem/rlh context。root element の phase 2 + 2.5 が終わった
        // 時点で `root_font_size` / `root_line_height` が確定するので、ここで
        // 初めて `Some` になる。`used_line_height_length` は
        // `crate::specified::SpecifiedValues::finalize_as_root` が自分の
        // `ctx` を組み立てるのに使う導出と同一 — 両者の一致は
        // `rlh_on_root_element_matches_child_root_line_height_basis` が check する。
        let child_ctx = match root_ctx {
            Some(ctx) => Some(ctx),
            None if is_element => Some(ResolveContext::with_root_line_height(
                computed.font_size,
                used_line_height_length(computed.line_height, computed.font_size),
            )),
            None => None,
        };

        // `::before`/`::after` — CSS Pseudo-Elements Module Level 4 §4
        // <https://drafts.csswg.org/css-pseudo-4/#treelike> (tree-abiding
        // pseudo-elements, of which `::before`/`::after` are a subcase):
        // "They inherit any inheritable properties from their originating
        // element; non-inheritable properties take their initial values as
        // usual." This is computed inline here, right where `id`'s own real
        // children would be, rather than in a separate pass after this
        // whole walk finishes, for one specific reason: `child_ctx` (the
        // `rem`/`rlh` basis `id`'s real children get) is *not* a
        // document-wide constant — a document can have multiple top-level
        // elements directly under the `Document` node, each independently
        // becoming its own `root_ctx == None` root with its own `rem` basis
        // (see `resolve_inheritance`'s `root_ctx` doc above). A pseudo's
        // `rem`/`rlh` basis must match whichever one its own real
        // originating element actually resolved against, and that value
        // only exists as this loop's *local* `child_ctx` at this exact
        // point — reconstructing it after the fact would require redoing
        // this same top-level-root bookkeeping in a second pass. Using
        // `computed` (not `parent_computed`) as the inherited-from base and
        // `child_ctx` (not `root_ctx`) as the absolutization context both
        // follow directly from that §4 inheritance framing — a pseudo is
        // one more entry alongside `id`'s real children in every sense this
        // cascade cares about, just one this function itself resolves
        // instead of pushing onto `stack` (there is no `StyleNodeId` for a
        // pseudo-element to push). Whether a box is actually generated from
        // the result — §4.1 <https://drafts.csswg.org/css-pseudo-4/#generated-content>'s
        // separate, `content`-conditioned rule, "When their computed
        // 'content' value is not 'none', these pseudo-elements generate
        // boxes as if they were immediate children of their originating
        // element" — is a downstream (`raikiri-dom`) decision this crate
        // does not make; see `CascadeResult::pseudo`'s doc.
        if is_element {
            for pseudo in [PseudoElem::Before, PseudoElem::After, PseudoElem::Marker] {
                let candidates = cascaded.pseudo_candidates(id, pseudo);
                let custom_candidates = cascaded.pseudo_custom_candidates(id, pseudo);
                if candidates.is_none() && custom_candidates.is_none() {
                    continue;
                }
                let pseudo_local_custom_properties = custom_candidates
                    .map(|candidates| resolve_custom_properties(&custom_properties, candidates));
                let pseudo_custom_properties = pseudo_local_custom_properties
                    .clone()
                    .unwrap_or_else(|| custom_properties.clone());
                let pseudo_local_custom_properties =
                    pseudo_local_custom_properties.unwrap_or_else(empty_custom_properties);

                let mut pseudo_specified = SpecifiedValues::inherit_from(&computed);
                if let Some(candidates) = candidates {
                    apply_winners(
                        candidates,
                        &mut winners,
                        &mut pseudo_specified,
                        &computed,
                        &pseudo_custom_properties,
                        None,
                        None,
                        None,
                    );
                }

                let ctx = child_ctx.expect(
                    "a generated pseudo-element candidate only exists for a \
                     StyleNodeKind::Element (is_element == true here, since \
                     only elements are ever matched by a selector — \
                     `collect_cascaded`'s pseudo-element pass runs inside \
                     the same `if let Some(elem) = node.as_element()` guard \
                     as its real-element pass), and `child_ctx` is `Some` \
                     for every element by this point in the match above",
                );
                let mut pseudo_computed = pseudo_specified.finalize(&computed, &ctx);
                pseudo_computed.custom_properties = pseudo_custom_properties;
                pseudo_computed.local_custom_properties = pseudo_local_custom_properties;
                pseudo_out.insert((id, pseudo), pseudo_computed);
            }
        }

        // out を id+1 サイズに resize してから index 書き込み。
        // `cascade()` の pre-allocation で通常 out.len() == node_count() のため
        // resize は no-op、defensive safety net として維持 (Dom impl の
        // node_count() 過小報告に対する保険)。
        let idx = id.0 as usize;
        if out.len() <= idx {
            out.resize(idx + 1, ComputedValues::initial());
        }
        out[idx] = computed.clone();
        if non_ua_margin_sides.len() <= idx {
            non_ua_margin_sides.resize(idx + 1, Sides::all(false));
        }
        non_ua_margin_sides[idx] = node_non_ua_margin;

        // 子を stack に push (own computed value を parent_computed として渡す)。
        // stack は LIFO なので document order で push するため reverse。
        // `child_ids` イテレータを直接 `stack` へ `extend` し、今回追加した
        // 末尾スライスだけを in-place `reverse()` する — 都度捨てる中間
        // `Vec` を経由しない。`child_ctx` は `Copy`
        // (`ResolveContext` の derive) なので closure 内で複数回使い回せる。
        //
        // なぜ document order を保つか: resolve_inheritance 自体の正しさも
        // 訪問順には依存しない — 各 node の computed 値は push 時点で既に
        // 確定している parent_computed / child_ctx だけから決まり、`winners`
        // scratch buffer は各 node の処理前後で完全に drain されるの
        // で兄弟の処理順に左右されない。ここで document order を維持して
        // いるのは refactor 前との**挙動の完全一致**のためであり、加えて
        // `winner_does_not_leak_into_next_sibling` 自身の doc comment が
        // 明記する「document order で先行する `<p>` → 後続 `<span>` の向き」
        // という leak 検出方向を、この traversal 順が引き続き満たすため。
        let start = stack.len();
        stack.extend(dom.child_ids(id).map(|child_id| {
            (
                child_id,
                computed.clone(),
                child_ctx,
                custom_properties.clone(),
            )
        }));
        stack[start..].reverse();
    }
}

/// 1 node 分の cascade winner を選び、staging 表現へ適用する (**phase 1**)。
///
/// `winners` は caller が walk loop の外で確保した scratch buffer。
/// 本関数が fill ([`pick_winners`]) と drain を対で
/// 行い、抜けるときは全 slot が `None` に戻っている。
///
/// # 適用順
///
/// slot を index 昇順 = [`PropertyKey`] の**宣言順**に走査する。
/// [`Option::take`] が slot を `None` に戻すので、この走査自体が次 node 用の
/// reset を兼ねる。
///
/// Shorthand keys must not reach this phase. CSS Cascading Level 4 §3
/// requires shorthand declarations to behave as if expanded in place, and
/// §6.1 determines the winner by order of appearance. Parsed entry points
/// expand shorthands before collecting candidates; the exhaustive expansion
/// match requires an explicit decision for new property variants.
///
/// # `candidates[winner.idx]` の unchecked index について
///
/// `winner.idx` は直前の [`pick_winners`] が **同じ `candidates`** に対して
/// 作ったものなので in-bounds。fill と drain が本関数 body 内で隣接しており、
/// 間に `candidates` を差し替える経路が無いことが根拠。
///
/// The index is valid only because [`pick_winners`] produced it from the
/// same candidate slice immediately before this drain. A stale slot could
/// otherwise select another declaration, so the winner buffer is reset for
/// every node.
///
/// # `candidates` の出所 (flat arena 化後)
///
/// 唯一の呼び出し元 [`resolve_inheritance`] は `candidates` を
/// [`CascadedArena::candidates`] からしか受け取らない。同メソッドは常に
/// 「その node 自身の区間ちょうど」の slice を返す設計になっており (fields
/// が private で他の切り出し方を作れない)、`winner.idx` が
/// **別 node の宣言を指す**という上記の危険が「slot leak (drain し損ね)」
/// 以外の経路 — 例えば `candidates` 自体が呼び出し側のミスで global index
/// space の slice になる — からは発生し得ない。
///
/// [`PropertyKey`]: crate::property::PropertyKey
// The winner application already groups several optional cascade side channels;
// the inherited computed values add one more required input for `inherit`
// resolution without changing that staging boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_winners(
    candidates: &[CascadedDecl],
    winners: &mut Vec<Option<RankedDecl>>,
    specified: &mut SpecifiedValues,
    inherited: &ComputedValues,
    custom_properties: &CustomPropertyEnvironment,
    mut page_value: Option<&mut crate::property::PageValue>,
    mut non_ua_margin_sides: Option<&mut Sides<bool>>,
    mut authored_writing_mode: Option<&mut Option<WritingMode>>,
) {
    pick_winners(candidates, winners);
    for slot in winners.iter_mut() {
        if let Some(winner) = slot.take() {
            let value = &candidates[winner.idx].0;
            if let Some(sides) = non_ua_margin_sides.as_deref_mut() {
                let non_ua = candidates[winner.idx].2 != Origin::UserAgent;
                match value.key() {
                    crate::property::PropertyKey::MarginTop => sides.top = non_ua,
                    crate::property::PropertyKey::MarginRight => sides.right = non_ua,
                    crate::property::PropertyKey::MarginBottom => sides.bottom = non_ua,
                    crate::property::PropertyKey::MarginLeft => sides.left = non_ua,
                    crate::property::PropertyKey::Margin
                    | crate::property::PropertyKey::MarginInline
                    | crate::property::PropertyKey::MarginBlock => {
                        sides.top = non_ua;
                        sides.right = non_ua;
                        sides.bottom = non_ua;
                        sides.left = non_ua;
                    }
                    _ => {}
                }
            }
            let value = match value {
                PropertyValue::BorderRadiusInherit => Some(PropertyValue::BorderRadius(
                    inherited_border_radius_value(inherited),
                )),
                PropertyValue::Deferred(deferred) => {
                    resolve_deferred_value(deferred, custom_properties)
                }
                _ => Some(value.clone()),
            };
            if let Some(value) = value {
                if let PropertyValue::WritingMode(mode) = &value
                    && let Some(slot) = authored_writing_mode.as_deref_mut()
                {
                    *slot = Some(*mode);
                }
                if let crate::property::PropertyValue::Page(page) = &value
                    && let Some(page_slot) = page_value.as_deref_mut()
                {
                    *page_slot = page.clone();
                }
                apply_value(value, specified);
            }
        }
    }
}

/// `font-weight` の specified value を computed absolute weight に解決する。
///
/// CSS Fonts 4 §2.2.1 "Relative Weights"
/// <https://www.w3.org/TR/css-fonts-4/#relative-weights> の
/// bolder / lighter table を **そのまま 6 行** で写したもの (`inherited` = 表の
/// `w`):
///
/// | inherited value | `bolder` | `lighter` |
/// |---|---|---|
/// | `w < 100`       | 400 | `w` (no change) |
/// | `100 <= w < 350`| 400 | 100 |
/// | `350 <= w < 550`| 700 | 100 |
/// | `550 <= w < 750`| 900 | 400 |
/// | `750 <= w < 900`| 900 | 700 |
/// | `900 <= w`      | `w` (no change) | 700 |
///
/// # 算術式で書いてはいけない理由
///
/// `min(w + 300, 900)` / `max(w - 300, 100)` のような近似は表の両端 2 行
/// ("no change" 行) を落とす。`<number [1,1000]>` の全域が author から到達
/// 可能になった今、その 2 行は実際に踏まれる:
///
/// - 親 `font-weight: 1000` + 子 `bolder` → **1000** (`900 <= w` 行の no change)。
///   `min(1300, 900)` なら誤って 900 に落ちる。
/// - 親 `font-weight: 50` + 子 `lighter` → **50** (`w < 100` 行の no change)。
///   `max(-250, 100)` なら誤って 100 に上がる。
///
/// なお表の境界は半開区間 (`350 <= w < 550` 等)。実装は match arm を上から順に
/// 評価させ、各 guard には **上限のみ** (`w < N`) を書く — 下限は直前 arm の
/// 否定として暗黙に成立する。したがって **arm の順序が spec の行順と 1 対 1** で
/// あることが正しさの条件であり、並べ替えは不可。
///
/// `pub(crate)` は他 module の doc からの intra-doc link のため — private 化で gate が red (規約 3)。
///
/// `inherited` / 戻り値は `f32` (`u16` から格上げ済み)。
/// table の境界値 (100 / 350 / 550 / 750 / 900) は全て整数だが、`inherited` は
/// fractional weight (`349.5` 等) を保持したまま渡ってくる。丸めずに直接
/// 比較するため行選択は spec §2.2.1 のとおり正確に決まる — 旧 `u16` 実装は
/// parse 段の丸めで `349.5` が `350` に化けてから本関数に渡り、`350 <= w < 550`
/// 行を誤って踏んでいた (詳細: [`crate::property`] の `parse_font_weight` doc)。
///
/// # 非有限 `inherited` (`NaN` / `±Inf`) — 本関数は guard しない
///
/// `u16` だった頃は非有限が型で構造的に排除されていたが、`f32` 化で
/// finiteness は「型で保証」から「呼び出し元の値
/// 検証で保証」に変わった。通常の cascade 経路は
/// [`crate::property`] の `parse_font_weight` の `[1, 1000]` range guard により
/// 常に finite だが、`ComputedValues` の field は全て `pub` で
/// [`crate::page::cascade_page`] も呼び出し側提供の
/// [`crate::page::PageInheritance`]`::FromRoot` を継承元 root として受け取るため、
/// cascade を経由しない直接構築
/// (`ComputedValues { font_weight: f32::NAN, .. }`) 経由で理論上到達しうる。
///
/// 両 arm とも `<` 比較は NaN に対し常に false になるが、catch-all arm の
/// 位置が異なるため結果は非対称: `Bolder` の catch-all は `w => w` (`900 <=
/// w` 行の no-change) なので `NaN` / `+Inf` は**そのまま伝播**する
/// (`-Inf` は最初の `w < 100.0` guard に一致し 400.0 に解決される)。
/// `Lighter` の catch-all は `_ => 700.0` なので `NaN` / `+Inf` は**700.0 に
/// 丸められる** (`-Inf` は同じく最初の guard に一致しそのまま伝播する)。
///
/// The resolver leaves non-finite and out-of-range values to the sink
/// boundary, where the value is normalized before it reaches layout.
pub(crate) fn resolve_relative_weight(specified: FontWeightValue, inherited: f32) -> f32 {
    match specified {
        FontWeightValue::Absolute(w) => w,
        FontWeightValue::Bolder => match inherited {
            w if w < 100.0 => 400.0,
            w if w < 350.0 => 400.0,
            w if w < 550.0 => 700.0,
            w if w < 750.0 => 900.0,
            w if w < 900.0 => 900.0,
            // `900 <= w`: no change (1000 のような 900 超の継承値をそのまま返す)
            w => w,
        },
        FontWeightValue::Lighter => match inherited {
            // `w < 100`: no change (50 のような 100 未満の継承値をそのまま返す)
            w if w < 100.0 => w,
            w if w < 350.0 => 100.0,
            w if w < 550.0 => 100.0,
            w if w < 750.0 => 400.0,
            w if w < 900.0 => 700.0,
            _ => 700.0,
        },
    }
}

/// `font-size` の `<relative-size>` (`larger` / `smaller`) を親の computed
/// font-size に対して解決する。[`resolve_relative_weight`] の font-size 版
/// (bolder/lighter と同型)。
///
/// CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop> verbatim:
///
/// > A `<relative-size>` keyword is interpreted relative to the computed
/// > font-size of the parent element and possibly the table of font sizes.
/// > \[…\] If the parent element has a keyword font size in the absolute size
/// > keyword mapping table, larger may compute the font size to the next
/// > entry in the table, and smaller may compute the font size to the
/// > previous entry in the table. \[…\] Instead of using next and previous
/// > items in the previous keyword table, User agents may instead use a
/// > simple ratio to increase or decrease the font size relative to the
/// > parent element. The specific ratio is unspecified, but should be around
/// > 1.2–1.5.
///
/// # next/previous table-entry 分岐を実装しない理由
///
/// spec は 2 分岐を "may" (どちらも規範ではなく許容) で並べており、raikiri は
/// simple-ratio 分岐**のみ**を実装する。`<absolute-size>` keyword は parser
/// (`parse_font_size_keyword`) が parse 時点で `medium` 基準の `Length::Px` に
/// 解決し尽くすため (variant 自体を保持しない)、継承された computed font-size
/// からは「親が keyword で指定したかどうか」を区別できず、table 分岐の前提
/// ("if the parent element has a keyword font size in the ... table") を
/// 安全に判定できない。
///
/// # ratio = 1.2 の根拠
///
/// spec 引用の "should be around 1.2–1.5" が許容 range の下限。**この 1.2 は
/// 同 §2.5.1 の note (CSS2 で隣接 index 間の scaling factor として 1.2 を
/// 採用したが小さいサイズで不足だったという指摘) とは別の根拠から来ている**
/// — 誤って note を典拠に引用しないこと (note は table 側の話で、本関数は
/// simple-ratio 分岐の話)。
pub(crate) fn resolve_relative_font_size(keyword: RelativeFontSize, inherited_px: f32) -> f32 {
    const RATIO: f32 = 1.2;
    match keyword {
        RelativeFontSize::Larger => inherited_px * RATIO,
        RelativeFontSize::Smaller => inherited_px / RATIO,
    }
}

/// specified value を継承元の computed values に対して解決し、**`PropertyValue`
/// 表現のまま** ([`ResolvedAgainstInherited`] に包んで) computed-equivalent な
/// 値を返す。
///
/// # なぜ [`apply_value`] と別に必要か
///
/// [`apply_value`] は解決結果を [`SpecifiedValues`] の field へ直接書き込むため、
/// 結果を `PropertyValue` として受け取りたい呼び手から reuse できない。
/// [`crate::page::cascade_page`] の結果は
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// という `HashMap<PropertyKey, PropertyValue>` として public に出るので、格納前に
/// この関数を通す必要がある。CSS Fonts 4 §2.2.1 の relative-weight table 自体は
/// `resolve_relative_weight` に 1 つしか存在せず、本関数と [`apply_value`] は
/// どちらもそこへ funnel する (table の二重実装は無い)。
///
/// # wildcard arm を置かない理由 (契約)
///
/// pass-through 側は全 variant を明示列挙し `_ => value` を使わない。これは意図的な
/// compile-time guard である: **継承元に依存する解決を持つ property を新しく足した
/// とき、`_` があると本関数を素通りして未解決値が public な結果に漏れる**。
/// The pass-through side is exhaustive rather than using a wildcard. Adding a
/// property variant therefore requires an explicit decision about whether it
/// depends on inherited values. This compile-time guard does not detect new
/// entry points or new payload semantics inside an existing variant.
///
/// # 本関数の pass-through は「解決済」ではない (phase 3 が要る)
///
/// 本関数が担うのは「継承元 computed values **だけ**で解ける」解決に限る =
/// **phase 2**。pass-through arm を通った値のうち、box property
/// ([`padding`](PropertyValue::PaddingTop) /
/// [`margin`](PropertyValue::MarginTop) / [`width`](PropertyValue::Width) /
/// [`height`](PropertyValue::Height) / `border-*-width`) と `line-height` の
/// [`Length`] `Em` / `Rem` / `Pt` は**まだ specified
/// 値**である。CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> の "Values in units of
/// em and ex are interpreted relative to the font associated with their
/// context" どおり `Em` は page context 自身の font に対する倍率であり、その
/// font-size は**同 cascade の兄弟 declaration から来得る**ため `inherited`
/// だけでは決まらない。`border-*-width` の style gating (CSS Backgrounds 3 §3.3)
/// も同様に兄弟 declaration (`border-*-style`) を要する。
///
/// **その解決は呼び手の責務である。** 唯一の呼び手
/// [`crate::page::cascade_page`] は本関数の直後に **phase 3**
/// ([`crate::page`] の `absolutize_in_page_context`) を走らせ、そこで page context の
/// font-size を基準に絶対化 + style gating を行う。
/// したがって
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// に届く時点では computed 値になっている — **本関数の戻り値をそのまま public に
/// 出す新しい呼び手を書いてはならない**。
///
/// element 経路の対応物は [`apply_value`] → [`SpecifiedValues::finalize`] で、
/// phase 2 (font-size 確定) → phase 3 (自 font-size 基準で残りを絶対化) が
/// 同じ順に走る。両経路の phase 3 は [`crate::resolve`] の同じ関数群へ funnel する
/// ので、spec 規則 (`em` / `rem` の基準、percentage の素通し、border style
/// gating) の実装は 1 本ずつしかない。
///
/// # `TextAlign::MatchParent` は本関数が解決する
///
/// [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent) は `inherited`
/// だけで解ける — CSS Text 3 §6.1 `#valdef-text-align-match-parent` の
/// 「実の親を持つ」半分 (root element の "computes to start" は対象外、下記注記)
/// — ので本関数の `TextAlign` arm が
/// [`crate::property::resolve_text_align_match_parent`] へ `inherited.text_align` +
/// `inherited.direction` を渡して解決する。以前は raikiri
/// が `direction` を computed 層に持たなかったため未実装だった。
///
/// ⚠️ **trap**: CSS Paged Media 3 §6 の "The page context inherits from the
/// root element" は「page context に親が無い」ことを意味**しない** —
/// `inherited` 引数は常に「実の親 (または L3 legacy exception の initial
/// values)」であり、[`TextAlign`](crate::property::TextAlign) doc が引用する
/// "Computes to start when specified on the root element" の特別扱いは
/// **本関数の対象外**。page context がその特別扱いを受けることは無い —
/// page context 自身が root element になるわけではないため。element 経路で
/// この特別扱いを担うのは [`SpecifiedValues::finalize_as_root`]。
///
/// The public contract is [`crate::PageCascadeResult::declarations`].
///
/// なお `Percent` は「未解決」ではない — box property の computed value は
/// percentage のままである (CSS Paged Media 3 §6 の "Percentage values on the
/// margin and padding properties are relative to the dimensions of the
/// containing block" = used 層の入力。引用は canonical 側)。element 経路の
/// [`crate::resolve::resolve_length_percentage`] と同じ扱い。
///
/// # 戻り値が生の [`PropertyValue`] ではなく [`ResolvedAgainstInherited`] な理由
///
/// 上記「この guard が守らない範囲」§2
/// (本関数を呼ばない新しい entry point) を型で狭めるため。詳細は
/// [`ResolvedAgainstInherited`] の doc を参照。
///
/// # `ctx` の caller contract
///
/// `ctx.root_line_height` は `inherited` から導出したもの
/// (`used_line_height_length(inherited.line_height, inherited.font_size)`)
/// を渡すこと — `FontSize` arm の `lh`/`rlh` 解決 (下記 arm 参照) がこの
/// 一致を前提にしている。唯一の呼び手 [`crate::page::cascade_page`] はこれを
/// 一度だけ構築し、本関数と phase 3 ([`crate::page`] の `absolutize_in_page_context`)
/// の両方に使い回す (`inherited` は関数全体で不変なので、二重に計算しても
/// 同じ値になる — 呼び手の doc 参照)。
pub(crate) fn inherited_border_radius(value: ComputedLengthPercentage) -> Length {
    match value {
        ComputedLengthPercentage::Px(px) => Length::Px(px),
        ComputedLengthPercentage::Percent(percent) => Length::Percent(percent),
    }
}

fn inherited_border_radius_value(inherited: &ComputedValues) -> BorderRadius {
    BorderRadius {
        top_left: inherited_border_radius(inherited.border_radius.top_left),
        top_right: inherited_border_radius(inherited.border_radius.top_right),
        bottom_right: inherited_border_radius(inherited.border_radius.bottom_right),
        bottom_left: inherited_border_radius(inherited.border_radius.bottom_left),
    }
}

fn inherited_margin_length(value: ComputedLengthPercentageOrAuto) -> LengthOrAuto {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => LengthOrAuto::Length(Length::Px(px)),
        ComputedLengthPercentageOrAuto::Percent(percent) => {
            LengthOrAuto::Length(Length::Percent(percent))
        }
        ComputedLengthPercentageOrAuto::Calc(calc) => LengthOrAuto::Calc(calc),
        ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
    }
}

pub(crate) fn resolve_against_inherited(
    value: PropertyValue,
    inherited: &ComputedValues,
    ctx: &ResolveContext,
) -> ResolvedAgainstInherited {
    ResolvedAgainstInherited(match value {
        PropertyValue::BorderRadiusInherit => {
            PropertyValue::BorderRadius(inherited_border_radius_value(inherited))
        },
        PropertyValue::MarginTopInherit => {
            PropertyValue::MarginTop(inherited_margin_length(inherited.margin.top))
        }
        PropertyValue::MarginRightInherit => {
            PropertyValue::MarginRight(inherited_margin_length(inherited.margin.right))
        }
        PropertyValue::MarginBottomInherit => {
            PropertyValue::MarginBottom(inherited_margin_length(inherited.margin.bottom))
        }
        PropertyValue::MarginLeftInherit => {
            PropertyValue::MarginLeft(inherited_margin_length(inherited.margin.left))
        }
        PropertyValue::MarginInherit => PropertyValue::Margin(Sides {
            top: inherited_margin_length(inherited.margin.top),
            right: inherited_margin_length(inherited.margin.right),
            bottom: inherited_margin_length(inherited.margin.bottom),
            left: inherited_margin_length(inherited.margin.left),
        }),
        // CSS Fonts 4 §2.2.1 "Relative Weights"
        // <https://www.w3.org/TR/css-fonts-4/#relative-weights>: `bolder` /
        // `lighter` は継承元の computed weight に対して解決される。ここで
        // `Absolute` に落とすので戻り値に relative keyword は残らない
        // (`Absolute(f32)` → `f32` → `Absolute(f32)` の round-trip は無損失)。
        PropertyValue::FontWeight(fw) => PropertyValue::FontWeight(FontWeightValue::Absolute(
            resolve_relative_weight(fw, inherited.font_weight),
        )),
        // `font-size` は継承元の computed font-size だけで解ける。
        // `crate::resolve::resolve_font_size` に funnel し、
        // 結果を `Length::Px` で包み直して computed-equivalent にする
        // (`FontWeight` arm が `Absolute(f32)` を返すのと同じ形)。
        //
        // 基準が `inherited.font_size` でよい根拠:
        //
        // - `em`: CSS Paged Media 3 §6 "Page Properties"
        //   <https://www.w3.org/TR/css-page-3/#page-properties> verbatim —
        //   "When used on the font-size property in the page context, they are
        //   relative to the font-size of the root element."
        //   `cascade_page` の `inherited` は root element の `ComputedValues`
        //   そのもの (`PageInheritance::LegacyInitialValues` なら initial) な
        //   ので、これが §6 の言う基準である。
        // - `rem`: CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>
        //   "Equal to the computed value of the em unit on the root element." —
        //   同じく `inherited.font_size`。
        // - `%`: **§6 は page context の `font-size` 上の `%` を規定していない。**
        //   CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>
        //   の "Percentages: refer to parent element's font size" と、§6 の
        //   "The page context inherits from the root element." を合わせると
        //   基準は root element の font-size になる、という**導出**であって
        //   §6 の明文ではない。
        // - `px` / `pt`: 絶対単位なので context 非依存。
        // - `lh` / `rlh`: §6 は `lh`/`rlh` を規定して
        //   いない。上記 `%` と同型の**導出** — 「the page context inherits
        //   from the root element」+ CSS Values 4 §6.1.1 の自己参照条項を
        //   合わせると、page context の self-reference basis (`lh` の基準)
        //   は root element の used line-height になる。`rlh` は page context
        //   にとって「root」と「parent」が同じ node (= `inherited`) なので
        //   両者は一致する — `crate::page::page_context_line_height_basis`
        //   の doc が `line-height` 自身の同じ状況について既に説明している
        //   のと同じ判断 (「両者が一致するのはこの page context に限った話」
        //   という同 doc の注記もそのまま当てはまる)。この一致のおかげで、
        //   `lh` の自己参照基準にも `ctx.root_line_height` をそのまま渡せる
        //   (上記「`ctx` の caller contract」節 — 呼び手が保証する)。
        //
        // 本 arm が `font-size` に限る理由: box property
        // (`padding` / `margin` / `width` / `height` / `border-*-width`) の
        // `em` は page context 自身の font-size を要し、それは同 cascade の兄弟
        // declaration から来得るので `inherited` だけでは決まらない。それらは
        // 呼び手 (`cascade_page`) が本関数の後に走らせる phase 3 の担当である
        // (上の「本関数の pass-through は『解決済』ではない」節を参照)。
        // **本 arm の戻り値が常に `Length::Px` であることは load-bearing** —
        // phase 3 はその値を page context の font-size (= `em` の基準) として
        // 読み戻す (`crate::page::page_context_font_size`)。次の `FontSizeRelative`
        // arm も同じ保証を守る (`PropertyValue::FontSize(Length::Px(_))` に収束させる)。
        PropertyValue::FontSize(len) => PropertyValue::FontSize(Length::Px(
            crate::resolve::resolve_font_size(len, inherited.font_size, ctx.root_line_height, ctx)
                .px(),
        )),
        // CSS Text 3 §6.1 `#valdef-text-align-match-parent`。
        // `inherited` は page context の inheritance parent (root element、
        // または L3 legacy exception の initial values) — 常に「実の親」扱いで
        // 解決する (上記 doc の trap 注記: page context 自身が root element の
        // "computes to start" 特別扱いを受けることは無い)。他 keyword は
        // no-op (関数 doc参照)。
        PropertyValue::TextAlign(t) => PropertyValue::TextAlign(
            resolve_text_align_internal_center(
                resolve_text_align_match_parent(t, inherited.text_align, inherited.direction),
                inherited.text_align,
            ),
        ),
        // CSS Fonts 4 §2.5 `<relative-size>` (`larger` / `smaller`):
        // `bolder` / `lighter` と同型、継承元の computed
        // font-size に対して解決する。`FontSize` variant に収束させる —
        // `PropertyValue::FontSizeRelative` doc の「解決タイミング」節が説明する
        // とおり、この variant は cascade winner の一時的な表現に留まり
        // public な結果 (`crate::page::PageCascadeResult::declarations`) には残らない。
        PropertyValue::FontSizeRelative(rel) => PropertyValue::FontSize(Length::Px(
            resolve_relative_font_size(rel, inherited.font_size.px()),
        )),
        // 本関数では解決しない property — pass-through。上記 doc の「本関数の
        // pass-through は『解決済』ではない (phase 3 が要る)」節が、これらを
        // 呼び手の phase 3 が絶対化することを説明している。`_` に潰さないこと。
        //
        // `Direction` はここに属する — computed value = specified value
        // (相対解決なし、`crate::property::Direction` doc 参照)、`Color` /
        // `FontFamily` と同型。
        v @ (PropertyValue::Color(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::LineHeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyleImage(_)
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::HangingPunctuation(_)
        | PropertyValue::Direction(_)
        | PropertyValue::TextIndent(_)
        | PropertyValue::PaddingTop(_)
        | PropertyValue::PaddingRight(_)
        | PropertyValue::PaddingBottom(_)
        | PropertyValue::PaddingLeft(_)
        | PropertyValue::Padding(_)
        // `padding-inline`/`padding-block` shorthand — same "nothing for
        // phase 2 to resolve" shape as `Padding`/`Margin` above (the
        // `<length-percentage>` font-relative absolutization happens later,
        // in phase 3).
        | PropertyValue::PaddingInline(_)
        | PropertyValue::PaddingBlock(_)
        | PropertyValue::MarginTop(_)
        | PropertyValue::MarginRight(_)
        | PropertyValue::MarginBottom(_)
        | PropertyValue::MarginLeft(_)
        | PropertyValue::Margin(_)
        | PropertyValue::MarginInline(_)
        | PropertyValue::MarginBlock(_)
        | PropertyValue::BorderTopWidth(_)
        | PropertyValue::BorderRightWidth(_)
        | PropertyValue::BorderBottomWidth(_)
        | PropertyValue::BorderLeftWidth(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::Border(_)
        // `border-style` / `border-width` / `border-color` shorthands —
        // same "nothing for phase 2 to resolve" shape as `Border` above
        // (non-inherited keywords/lengths; structurally unreachable here
        // since rule.rs expands them first).
        | PropertyValue::BorderStyle(_)
        | PropertyValue::BorderWidth(_)
        | PropertyValue::BorderColor(_)
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::MaxWidth(_)
        | PropertyValue::MaxHeight(_)
        | PropertyValue::MinWidth(_)
        | PropertyValue::MinHeight(_)
        | PropertyValue::MinBlockSize(_)
        | PropertyValue::Top(_)
        | PropertyValue::Right(_)
        | PropertyValue::Bottom(_)
        | PropertyValue::Left(_)
        | PropertyValue::BoxSizing(_)
        // `overflow-x`/`overflow-y`/`overflow` join this arm — CSS Overflow 3
        // §3.1's cross-axis coupling (`resolve_overflow`) depends only on the
        // *other axis of the same node*, never on the inheritance parent, so
        // there is nothing for this function (phase 2) to resolve. It is
        // applied in phase 3 instead (`crate::page::absolutize_in_page_context`,
        // mirroring the element path's `SpecifiedValues::absolutize_with`).
        | PropertyValue::OverflowX(_)
        | PropertyValue::OverflowY(_)
        | PropertyValue::Overflow(_)
        // `writing-mode` joins this arm for the same reason `overflow-x`/
        // `overflow-y`/`overflow` do — its `HorizontalTb` collapse
        // (`resolve_writing_mode`, CSS Writing Modes 4 §3.2) depends only on
        // its own specified value, never on the inheritance parent, so there
        // is nothing for this function (phase 2) to resolve. It is applied
        // in phase 3 instead (`crate::page::absolutize_in_page_context`,
        // mirroring the element path's `SpecifiedValues::absolutize_with`).
        // See `WritingMode` doc's Non-goal section.
        | PropertyValue::WritingMode(_)
        | PropertyValue::RubyPosition(_)
        // `text-decoration-line`/`-style`/`-color` (and the `text-decoration`
        // shorthand, structurally unreachable here per
        // `crate::rule::expand_shorthand_into`) carry no length and do not
        // depend on the inheritance parent (computed value = specified
        // keyword(s)/color, `TextDecorationLine`/`TextDecorationStyle`/
        // `TextDecorationColor` docs) — nothing for phase 2 to resolve.
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextUnderlinePosition(_)
        | PropertyValue::TextDecoration(_)
        // `text-decoration-thickness` / `text-decoration-inset` carry a
        // `<length-percentage>` that needs the *declaring node's own*
        // font-size (phase 3), not the inheritance parent's — same shape
        // as `Padding`/`Margin`/`Width` above, nothing for phase 2 to
        // resolve here. (`TextDecoration` shorthand itself joins the group
        // above since expansion removes it before this function runs.)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextDecorationInset(_)
        | PropertyValue::TextUnderlineOffset(_)
        // `vertical-align`'s 6 keywords (`baseline`/`sub`/`super`/`middle`/
        // `text-top`/`text-bottom`) describe a shift *relative to the
        // parent's font metrics*, but that relation is a used-value/layout
        // concern (raikiri-paint scope, `VerticalAlign` doc's "baseline
        // shift 量の計算は raikiri-paint scope" note) — CSS 2.1 §10.8.1's
        // computed value for these keywords is still the bare specified
        // keyword, so there is nothing for this function (phase 2,
        // inheritance-parent-relative resolution) to resolve. The
        // `<length>` variant needs *own-node* font-size resolution (`em`/
        // `rem`), which is phase 3's job
        // (`crate::specified::SpecifiedValues::absolutize_with`'s
        // `resolve_vertical_align` call, same split `FlexBasis` below
        // uses) — not this function's, since it depends on the declaring
        // node's own winners rather than the inheritance parent.
        | PropertyValue::VerticalAlign(_)
        // `font-style` carries no length at this crate's scope
        // (`normal`/`italic`/`oblique` implemented, `oblique`'s `<angle>`
        // argument is not — `FontStyle` doc) and does not depend on the
        // inheritance parent — nothing for phase 2 to resolve.
        | PropertyValue::FontStyle(_)
        // `text-transform` carries no length (`TextTransform` doc) and
        // does not depend on the inheritance parent — nothing for phase 2
        // to resolve.
        | PropertyValue::TextTransform(_)
        // `visibility` carries no length (`Visibility` doc) and does not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::Visibility(_)
        // `z-index` carries no length (`ZIndexValue` doc) and does not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::ZIndex(_)
        // `word-break` (CSS Text 3 §5.1) carries no length (`WordBreak`
        // doc) and does not depend on the inheritance parent — nothing for
        // phase 2 to resolve.
        | PropertyValue::WordBreak(_)
        // `overflow-wrap`/`word-wrap` (CSS Text 3 §5.4) carries no length
        // (`OverflowWrap` doc) either — same as `WordBreak` above.
        | PropertyValue::OverflowWrap(_)
        // `letter-spacing` / `word-spacing` carry a `<length>` that needs
        // the *declaring node's own* font-size (phase 3), not the
        // inheritance parent's — same shape as `Padding`/`Margin`/`Width`/
        // `Height` above, nothing for phase 2 to resolve here.
        | PropertyValue::LetterSpacing(_)
        | PropertyValue::WordSpacing(_)
        // `tab-size`'s `<length>` alternative needs the same *declaring
        // node's own* font-size basis (phase 3) as `LetterSpacing`/
        // `WordSpacing` above; its `<number>` alternative carries no length
        // at all. Either way, nothing for phase 2 to resolve here.
        | PropertyValue::TabSize(_)
        // `break-before`/`break-after` (CSS Fragmentation Module Level 3
        // §3.1, legacy shorthand `page-break-before`/`page-break-after`
        // included) carry no length (`BreakBetween` doc) and do not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        // `break-inside` (CSS Fragmentation Module Level 3 §3.2, legacy
        // shorthand `page-break-inside` included) carries no length
        // either (`BreakInside` doc) — same as `BreakBefore`/`BreakAfter`
        // above.
        | PropertyValue::BreakInside(_)
        // `float` (CSS2 §9.5.1) carries no length and does not depend on
        // the inheritance parent — nothing for phase 2 to resolve. The
        // §9.7 `display` coupling this value drives is a same-node
        // dependency, resolved in phase 3
        // (`crate::specified::SpecifiedValues::absolutize_with`'s
        // `resolve_display_for_float` call), not here.
        | PropertyValue::Float(_)
        // `clear` (CSS2 §9.5.2) carries no length either — same as
        // `Float` above.
        | PropertyValue::Clear(_)
        // `white-space` (CSS Text 3 §3) carries no length (`WhiteSpace`
        // doc) and does not depend on the inheritance parent — nothing for
        // phase 2 to resolve.
        | PropertyValue::WhiteSpace(_)
        // `text-wrap` (CSS Text 4 §5 subset) carries no length and does not
        // depend on the inheritance parent — nothing for phase 2 to resolve.
        | PropertyValue::TextWrap(_)
        // `flex-*` / alignment / `row-gap`/`column-gap` (and their
        // shorthands) — same "nothing for phase 2 to resolve" shape as
        // `Padding`/`Margin`/`Width`/`Height` above for the length-bearing
        // ones (`FlexBasis`/`RowGap`/`ColumnGap`/`Flex`/`Gap`; phase 3
        // (`absolutize_in_page_context`) does the declaring node's own
        // font-size resolution), and no length payload at all for the rest
        // (`FlexDirection`/`FlexWrap`/`FlexGrow`/`FlexShrink`/
        // `JustifyContent`/`AlignContent`/`AlignItems`/`AlignSelf`/
        // `PlaceContent`).
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        | PropertyValue::FlexBasis(_)
        | PropertyValue::Flex(_)
        | PropertyValue::FlexFlow(_)
        | PropertyValue::Order(_)
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        | PropertyValue::RowGap(_)
        | PropertyValue::ColumnGap(_)
        | PropertyValue::Gap(_)
        | PropertyValue::PlaceContent(_)
        // `hyphens` (CSS Text 3 §5.3) carries no length (`Hyphens` doc) and
        // does not depend on the inheritance parent — nothing for phase 2
        // to resolve.
        | PropertyValue::Hyphens(_)
        // `font-variant-caps` (CSS Fonts Module Level 3 §6.6) carries no
        // length (`FontVariantCaps` doc) and does not depend on the
        // inheritance parent — nothing for phase 2 to resolve, same shape
        // as `FontStyle` above.
        | PropertyValue::FontVariantCaps(_)
        // `quotes` (CSS Content 3 §2.4.1) carries no length and does not
        // depend on the inheritance parent (computed value = specified
        // value, `ComputedValues::quotes` doc) — nothing for phase 2 to
        // resolve.
        | PropertyValue::Quotes(_)
        // `text-shadow`'s per-item lengths (offset-x/offset-y/blur-radius)
        // need the *declaring node's own* font-size (phase 3), not the
        // inheritance parent's — same shape as `LetterSpacing`/`WordSpacing`
        // above, nothing for phase 2 to resolve here. The `<color>`
        // component carries no length either (`TextShadowColor` doc).
        | PropertyValue::TextShadow(_)
        // F7/F8/F9 lengths need the declaring node/page context and therefore
        // remain for phase 3.
        | PropertyValue::BorderRadius(_)
        | PropertyValue::BorderRadiusTopLeft(_)
        | PropertyValue::BorderRadiusTopRight(_)
        | PropertyValue::BorderRadiusBottomRight(_)
        | PropertyValue::BorderRadiusBottomLeft(_)
        | PropertyValue::BoxShadow(_)
        | PropertyValue::Outline(_)
        | PropertyValue::OutlineWidth(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
        | PropertyValue::OutlineOffset(_)
        // CSS Grid Layout Module Level 1 grid-template-columns/-rows/-areas
        // (§7.2/§7.3) + grid-auto-columns/-rows/-flow (§7.6/§7.7) +
        // grid-row-start/-end/grid-column-start/-end (+ their `grid-row`/
        // `grid-column` shorthands, §8.3/§8.4) — same "nothing for phase 2
        // to resolve" shape as `FlexBasis`/`RowGap` above for the
        // length-bearing track-sizing ones (`GridTemplateColumns`/
        // `GridTemplateRows`/`GridAutoColumns`/`GridAutoRows`; phase 3 does
        // the declaring node's own font-size resolution over the track
        // list), and no length payload at all for the rest
        // (`GridTemplateAreas`/`GridAutoFlow`/`GridRowStart`/`GridRowEnd`/
        // `GridColumnStart`/`GridColumnEnd`/`GridRow`/`GridColumn`).
        | PropertyValue::GridTemplateColumns(_)
        | PropertyValue::GridTemplateRows(_)
        | PropertyValue::GridTemplateAreas(_)
        | PropertyValue::GridAutoColumns(_)
        | PropertyValue::GridAutoRows(_)
        | PropertyValue::GridAutoFlow(_)
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        | PropertyValue::GridRow(_)
        | PropertyValue::GridColumn(_)
        // justify-items (CSS Box Alignment Module Level 3 §7.1) /
        // justify-self (§6.1) / place-items (§7.3) / place-self (§6.3) —
        // same "no length payload" shape as `AlignItems`/`AlignSelf` above.
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        | PropertyValue::PlaceItems(_)
        | PropertyValue::PlaceSelf(_)
        // `orphans`/`widows` (CSS Fragmentation Module Level 3 §3.3) carry a
        // bare positive `<integer>`, not a length — nothing for phase 2 to
        // resolve, same shape as `FlexGrow`/`FlexShrink` above.
        | PropertyValue::Orphans(_)
        | PropertyValue::Widows(_)
        // background-repeat / background-attachment / background-clip /
        // background-origin / background-size / background-position (CSS
        // Backgrounds and Borders 3 §2.4-§2.9) — nothing for phase 2 to
        // resolve against the inheritance parent (all 6 are non-inherited,
        // `background-color`'s sibling shape); `background-size`/
        // `background-position`'s `<length-percentage>` absolutization is
        // phase 3's job (`crate::specified::SpecifiedValues::absolutize_with`
        // on the element path, `crate::page`'s `absolutize_in_page_context`
        // on the page path), same as `Padding`/`Width` above.
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)
        | PropertyValue::BackgroundSize(_)
        | PropertyValue::BackgroundPosition(_)
        // background-image (CSS Backgrounds and Borders 3 §2.3) — `None` /
        // `Url(String)` are opaque to font-size/line-height resolution, same
        // as its 4 keyword-only siblings just above. `Gradient(..)` *does*
        // carry `<length-percentage>`/`<angle>` payloads (CSS Images 4 §3),
        // but this crate deliberately never resolves them against the
        // inheritance parent (or at all, pre-paint) — resolving a gradient's
        // lengths needs the gradient box's own dimensions, an input phase 2
        // doesn't have, so the whole variant is left as specified-layer data
        // all the way into `ComputedValues` (`BackgroundImage` doc's scope
        // note). Nothing for phase 2 to do here either way.
        | PropertyValue::BackgroundImage(_)
        // `background` shorthand — same "nothing for phase 2 to resolve"
        // shape as `Padding`/`Margin`/`Border`/`Flex`/`Gap` above: its
        // `<length-percentage>` components (`position`/`size`) need the
        // *declaring node's own* font-size (phase 3), not the inheritance
        // parent's, and it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::Background(_)
        // `font` shorthand — same "nothing for phase 2 to resolve" shape as
        // `Padding`/`Margin`/`Border`/`Flex`/`Gap`/`Background` above: its
        // length-bearing components (`size` as <length-percentage>,
        // `line-height` as <number>/<length-percentage>) need the
        // *declaring node's own* font-size (phase 3), not the inheritance
        // parent's, and it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::Font(_)
        // object-fit (CSS Images Module Level 3 §5.1) — non-inherited,
        // keyword-only, same "nothing for phase 2 to resolve" shape as
        // `BackgroundRepeat` above.
        | PropertyValue::ObjectFit(_)
        // object-position (CSS Images Module Level 3 §5.2) — non-inherited,
        // reuses `CssPosition` (`background-position`'s type); its
        // `<length-percentage>` absolutization is phase 3's job, same as
        // `BackgroundPosition` above.
        | PropertyValue::ObjectPosition(_)
        // opacity (CSS Color 4 §3.3) — non-inherited, carries a bare
        // `<number>`/`<percentage>`-derived `f32`, not a length, and does
        // not depend on the inheritance parent — nothing for phase 2 to
        // resolve here. The `[0,1]` clamp (CSS Color 4 §3.3's "computed
        // value: … clamped" rule) is phase 3's job
        // (`crate::specified::SpecifiedValues::absolutize_with`), same
        // split `FlexGrow`/`ZIndex` use for their own phase-3-only work.
        | PropertyValue::Opacity(_)
        // isolation (CSS Compositing and Blending Level 1 §3.4.2) /
        // mix-blend-mode (§3.4.1) — both non-inherited, bare keyword
        // payloads with no phase-2 dependency, same shape as `ObjectFit`
        // above.
        | PropertyValue::Isolation(_)
        | PropertyValue::MixBlendMode(_)
        // mask-image (CSS Masking Level 1 §7.1) / clip-path (§5.1) —
        // non-inherited, `<url>`/`<gradient>`/`<geometry-box>` payloads
        // that don't depend on the inheritance parent — nothing for phase
        // 2 to resolve here, same as `BackgroundImage` above (neither
        // crate absolutizes their embedded lengths at all — see
        // `MaskImage`/`ClipPath` doc's scope notes).
        | PropertyValue::MaskImage(_)
        | PropertyValue::ClipPath(_)
        // transform (CSS Transforms Level 1 §4) / filter (CSS Filter
        // Effects Level 1 §5) — non-inherited, embedded `Length`/`Angle`/
        // `f32` payloads that don't depend on the inheritance parent —
        // nothing for phase 2 to resolve here, same as `MaskImage`/
        // `ClipPath` above (neither absolutizes at all, see
        // `TransformFunction`/`FilterFunction` doc's scope notes).
        | PropertyValue::Transform(_)
        | PropertyValue::Filter(_)
        | PropertyValue::TableLayout(_)
        | PropertyValue::BorderCollapse(_)
        // `border-spacing` (CSS Tables 3 §6.1) — `<length>{1,2}` の絶対化は
        // 宣言 node 自身の font-size を要するため phase 3 の仕事
        // (`Padding`/`Margin` arm と同じ "nothing for phase 2" 形)。
        // `caption-side` (§7) / `empty-cells` (§8) は bare keyword payload
        // のため phase 2 依存なし (`BorderCollapse` と同じ)。
        | PropertyValue::BorderSpacing(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_)
        | PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::CalcLengthPercentage { .. }
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(_)
        | PropertyValue::Columns(_)) => v,
    })
}

/// [`resolve_against_inherited`] (phase 2) を通過済であることを **型で**示す
/// wrapper。tuple field は本 module (`cascade`) に private — 他 module は
/// [`resolve_against_inherited`] を呼ぶ以外にこの型の値を作れない。
///
/// [`crate::page`] の `absolutize_in_page_context` (phase 3) は引数にこの型を
/// 要求するので、page 経路で phase 3 を再利用する限り、呼び手がどの module に
/// 書かれていても [`resolve_against_inherited`] を経由せざるを得ない —
/// `page` module 自身も、本型が `cascade` module 定義である以上、tuple field
/// に対しては他の module と同じ「foreign」な立場になる (`page` は単に
/// `cascade` と別の module であり、それ以上の特別扱いは無い)。
///
/// # narrowed, not closed
///
/// 本 module (`cascade.rs`) 自身に新しい経路が追加された場合はこの限りでは
/// ない (tuple field は定義 module 内では直接見える) し、margin-box cascade
/// が phase 3 を再利用せず独自の絶対化ロジックを書けばこの型は何も強制しない
/// — 残る「経路の数え上げ」不能性は、別途 margin-box cascade 実装の
/// acceptance criteria として切り出してある。[`resolve_against_inherited`]
/// の doc「この guard が守らない範囲」§2 も参照。
///
/// This wrapper marks values that have passed phase 2 before phase 3.
/// The direct constructor exists only in test builds; production code uses
/// [`resolve_against_inherited`].
#[derive(Debug)]
pub(crate) struct ResolvedAgainstInherited(PropertyValue);

impl ResolvedAgainstInherited {
    /// Phase 2 を通過済の値を取り出す (所有権ごと)。
    pub(crate) fn into_property_value(self) -> PropertyValue {
        self.0
    }

    /// Phase 2 を通過済の値を覗き見る (所有権を取らない版)。[`crate::page`] の
    /// `page_context_font_size` / `page_context_border_styles` が、phase 3 に
    /// 渡す前の `font-size` / `border-*-style` を読むために使う。
    pub(crate) fn as_property_value(&self) -> &PropertyValue {
        &self.0
    }

    /// Test-only constructor for phase-3 inputs.
    #[cfg(test)]
    pub(crate) fn for_test(value: PropertyValue) -> Self {
        Self(value)
    }
}

/// Cascade winner 1 つを staging 表現 ([`SpecifiedValues`]) に書き込む
/// (**phase 1**)。
///
/// length を運ぶ property は specified 表現のまま格納し、絶対化は
/// [`SpecifiedValues::finalize`] に任せる (`em` などの基準 font-size は、その node の
/// 全 winner を適用し終えるまで確定しない)。winner の適用順は任意なので、他
/// property の値に依存する解決 (`text-align: match-parent` など) もここでは行わない。
///
/// 例外は `font-weight` の `bolder` / `lighter` と `font-size` の `larger` /
/// `smaller`。`target` は [`SpecifiedValues::inherit_from`] で親の computed 値に
/// seed 済みなので、これらの arm は上書き前に `target` から継承値を読んで絶対値に
/// 解決する。
///
/// shorthand の arm は、[`crate::rule::expand_shorthand_into`] が cascade 前に
/// longhand へ展開するため通常は到達しない。直接呼ばれた場合は cascade と同じ
/// `crate::rule` の per-family expander (例: [`crate::rule::expand_border`]) に委譲する。
///
/// `pub(crate)` は他 module の doc からの intra-doc link のため。
pub(crate) fn apply_value(value: PropertyValue, target: &mut SpecifiedValues) {
    match value {
        PropertyValue::Color(c) => target.color = c,
        PropertyValue::BackgroundColor(c) => target.background_color = c,
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        PropertyValue::FontSizeRelative(rel) => {
            // Always the `Length::Px` parent seed: `FontSize` shares this
            // key's single winner slot, so it has not overwritten the seed.
            let inherited_px = target.font_size.payload();
            target.font_size = Length::Px(resolve_relative_font_size(rel, inherited_px));
        }
        PropertyValue::FontWeight(fw) => {
            let inherited = target.font_weight;
            target.font_weight = resolve_relative_weight(fw, inherited);
        }
        PropertyValue::LineHeight(lh) => target.line_height = lh,
        PropertyValue::Display(d) => target.display = d,
        PropertyValue::ListStyleType(v) => target.list_style_type = v,
        PropertyValue::ListStyleImage(v) => target.list_style_image = v,
        PropertyValue::ListStylePosition(v) => target.list_style_position = v,
        PropertyValue::CounterReset(v) => target.counter_reset = v,
        PropertyValue::CounterResetInherit => {
            target.counter_reset = crate::property::empty_counter_entries();
        }
        PropertyValue::CounterIncrement(v) => target.counter_increment = v,
        PropertyValue::CounterSet(v) => target.counter_set = v,
        PropertyValue::Content(v) => target.content = v,
        PropertyValue::StringSet(v) => target.string_set = v,
        PropertyValue::Position(pv) => match pv {
            PositionValue::Static => {
                target.position = PositionValue::Static;
            }
            PositionValue::Sticky => {
                target.position = PositionValue::Sticky;
            }
            PositionValue::Relative => {
                target.position = PositionValue::Relative;
            }
            PositionValue::Absolute => {
                target.position = PositionValue::Absolute;
            }
            PositionValue::Fixed => {
                target.position = PositionValue::Fixed;
            }
            PositionValue::Running(name) => {
                target.running_templates.push(RunningTemplate { name });
            }
        },
        PropertyValue::TextAlign(t) => target.text_align = t,
        PropertyValue::HangingPunctuation(v) => target.hanging_punctuation = v,
        PropertyValue::TextJustify(v) => target.text_justify = v,
        PropertyValue::TextAlignLast(v) => target.text_align_last = v,
        PropertyValue::TextIndent(v) => {
            target.text_indent = v.length;
            target.text_indent_ch_factor = match v.length {
                Length::Ch(factor) if factor.is_finite() => Some(factor),
                _ => None,
            };
            target.text_indent_ch_font = None;
            target.text_indent_ch_inherited = false;
            target.text_indent_hanging = v.hanging;
            target.text_indent_each_line = v.each_line;
        }
        PropertyValue::Direction(d) => target.direction = d,
        PropertyValue::PaddingTop(v) => target.padding.top = v,
        PropertyValue::PaddingRight(v) => target.padding.right = v,
        PropertyValue::PaddingBottom(v) => target.padding.bottom = v,
        PropertyValue::PaddingLeft(v) => target.padding.left = v,
        PropertyValue::Padding(sides) => expand_padding(sides, |v| apply_value(v, target)),
        PropertyValue::PaddingInline(pair) => {
            expand_padding_inline(pair, |v| apply_value(v, target))
        }
        PropertyValue::PaddingBlock(pair) => expand_padding_block(pair, |v| apply_value(v, target)),
        // Page-cascade-only inherit markers; the element parser never produces them.
        PropertyValue::BorderRadiusInherit
        | PropertyValue::MarginTopInherit
        | PropertyValue::MarginRightInherit
        | PropertyValue::MarginBottomInherit
        | PropertyValue::MarginLeftInherit
        | PropertyValue::MarginInherit => {}
        PropertyValue::MarginTop(v) => target.margin.top = v,
        PropertyValue::MarginRight(v) => target.margin.right = v,
        PropertyValue::MarginBottom(v) => target.margin.bottom = v,
        PropertyValue::MarginLeft(v) => target.margin.left = v,
        PropertyValue::Margin(sides) => expand_margin(sides, |v| apply_value(v, target)),
        PropertyValue::MarginInline(pair) => expand_margin_inline(pair, |v| apply_value(v, target)),
        PropertyValue::MarginBlock(pair) => expand_margin_block(pair, |v| apply_value(v, target)),
        PropertyValue::BorderTopWidth(v) => target.border.top.width = v,
        PropertyValue::BorderRightWidth(v) => target.border.right.width = v,
        PropertyValue::BorderBottomWidth(v) => target.border.bottom.width = v,
        PropertyValue::BorderLeftWidth(v) => target.border.left.width = v,
        PropertyValue::BorderTopStyle(v) => target.border.top.style = v,
        PropertyValue::BorderRightStyle(v) => target.border.right.style = v,
        PropertyValue::BorderBottomStyle(v) => target.border.bottom.style = v,
        PropertyValue::BorderLeftStyle(v) => target.border.left.style = v,
        PropertyValue::BorderTopColor(v) => target.border.top.color = v,
        PropertyValue::BorderRightColor(v) => target.border.right.color = v,
        PropertyValue::BorderBottomColor(v) => target.border.bottom.color = v,
        PropertyValue::BorderLeftColor(v) => target.border.left.color = v,
        PropertyValue::Border(sides) => expand_border(sides, |v| apply_value(v, target)),
        PropertyValue::CalcLengthPercentage { key, value } => match key {
            crate::property::PropertyKey::Width => target.width = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::Height => target.height = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::MaxWidth => target.max_width = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::MaxHeight => {
                target.max_height = LengthOrAuto::Calc(value)
            }
            crate::property::PropertyKey::MinWidth => target.min_width = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::MinHeight => {
                target.min_height = LengthOrAuto::Calc(value)
            }
            crate::property::PropertyKey::MinBlockSize => {
                target.min_block_size = Some(LengthOrAuto::Calc(value))
            }
            crate::property::PropertyKey::Top => target.top = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::Right => target.right = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::Bottom => target.bottom = LengthOrAuto::Calc(value),
            crate::property::PropertyKey::Left => target.left = LengthOrAuto::Calc(value),
            _ => {}
        },
        PropertyValue::Width(v) => target.width = v,
        PropertyValue::Height(v) => target.height = v,
        PropertyValue::MaxWidth(v) => target.max_width = v,
        PropertyValue::MaxHeight(v) => target.max_height = v,
        PropertyValue::MinWidth(v) => target.min_width = v,
        PropertyValue::MinHeight(v) => target.min_height = v,
        PropertyValue::MinBlockSize(v) => target.min_block_size = Some(v),
        PropertyValue::Top(v) => target.top = v,
        PropertyValue::Right(v) => target.right = v,
        PropertyValue::Bottom(v) => target.bottom = v,
        PropertyValue::Left(v) => target.left = v,
        PropertyValue::BoxSizing(bs) => target.box_sizing = bs,
        PropertyValue::OverflowX(v) => target.overflow.x = v,
        PropertyValue::OverflowY(v) => target.overflow.y = v,
        PropertyValue::Overflow(pair) => expand_overflow(pair, |v| apply_value(v, target)),
        PropertyValue::TextDecorationLine(v) => target.text_decoration_line = v,
        PropertyValue::TextDecorationStyle(v) => target.text_decoration_style = v,
        PropertyValue::TextDecorationColor(v) => target.text_decoration_color = v,
        PropertyValue::TextDecorationInset(v) => target.text_decoration_inset = v,
        PropertyValue::TextUnderlineOffset(v) => target.text_underline_offset = v,
        PropertyValue::TextDecoration(shorthand) => {
            expand_text_decoration(shorthand, |v| apply_value(v, target))
        }
        PropertyValue::VerticalAlign(va) => target.vertical_align = va,
        PropertyValue::FontStyle(fs) => target.font_style = fs,
        PropertyValue::TextTransform(tt) => target.text_transform = tt,
        PropertyValue::Visibility(v) => target.visibility = v,
        PropertyValue::ZIndex(z) => target.z_index = z,
        PropertyValue::WordBreak(wb) => target.word_break = wb,
        PropertyValue::LineBreak(lb) => target.line_break = lb,
        PropertyValue::OverflowWrap(ow) => target.overflow_wrap = ow,
        PropertyValue::LetterSpacing(ls) => {
            target.letter_spacing = ls;
            target.letter_spacing_ch_factor = match ls {
                LengthOrNormal::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                _ => None,
            };
        }
        PropertyValue::WordSpacing(ws) => {
            target.word_spacing = ws;
            target.word_spacing_ch_factor = match ws {
                LengthOrNormal::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                _ => None,
            };
        }
        PropertyValue::TabSize(ts) => target.tab_size = ts,
        PropertyValue::BreakBefore(bb) => target.break_before = bb,
        PropertyValue::BreakAfter(bb) => target.break_after = bb,
        PropertyValue::BreakInside(bi) => target.break_inside = bi,
        PropertyValue::Float(f) => target.float = f,
        PropertyValue::Clear(c) => target.clear = c,
        PropertyValue::WhiteSpace(ws) => target.white_space = ws,
        PropertyValue::TextWrap(v) => target.text_wrap = v,
        PropertyValue::Hyphens(h) => target.hyphens = h,
        PropertyValue::FlexDirection(fd) => target.flex_direction = fd,
        PropertyValue::FlexWrap(fw) => target.flex_wrap = fw,
        PropertyValue::FlexGrow(g) => target.flex_grow = g,
        PropertyValue::FlexShrink(s) => target.flex_shrink = s,
        PropertyValue::FlexBasis(fb) => target.flex_basis = fb,
        PropertyValue::Flex(f) => expand_flex(f, |v| apply_value(v, target)),
        PropertyValue::FlexFlow(f) => expand_flex_flow(f, |v| apply_value(v, target)),
        PropertyValue::Order(o) => target.order = o,
        PropertyValue::JustifyContent(jc) => target.justify_content = jc,
        PropertyValue::AlignContent(ac) => target.align_content = ac,
        PropertyValue::AlignItems(ai) => target.align_items = ai,
        PropertyValue::AlignSelf(as_) => target.align_self = as_,
        PropertyValue::RowGap(rg) => target.row_gap = rg,
        PropertyValue::ColumnGap(cg) => target.column_gap = cg,
        PropertyValue::Gap(g) => expand_gap(g, |v| apply_value(v, target)),
        PropertyValue::PlaceContent(p) => expand_place_content(p, |v| apply_value(v, target)),
        PropertyValue::FontVariantCaps(fvc) => target.font_variant_caps = fvc,
        PropertyValue::Quotes(v) => {
            target.quotes_auto = false;
            target.quotes = v;
        }
        PropertyValue::TextShadow(shadows) => target.text_shadow = shadows,
        PropertyValue::BorderRadius(v) => target.border_radius = v,
        PropertyValue::BorderRadiusTopLeft(v) => target.border_radius.top_left = v,
        PropertyValue::BorderRadiusTopRight(v) => target.border_radius.top_right = v,
        PropertyValue::BorderRadiusBottomRight(v) => target.border_radius.bottom_right = v,
        PropertyValue::BorderRadiusBottomLeft(v) => target.border_radius.bottom_left = v,
        PropertyValue::BoxShadow(shadows) => target.box_shadow = shadows,
        PropertyValue::Outline(outline) => expand_outline(outline, |v| apply_value(v, target)),
        PropertyValue::OutlineWidth(v) => target.outline.width = v,
        PropertyValue::OutlineStyle(v) => target.outline.style = v,
        PropertyValue::OutlineColor(v) => target.outline.color = v,
        PropertyValue::OutlineOffset(v) => target.outline_offset = v,
        PropertyValue::GridArea(area) => {
            target.grid_row_start = area.row_start;
            target.grid_column_start = area.column_start;
            target.grid_row_end = area.row_end;
            target.grid_column_end = area.column_end;
        }
        PropertyValue::Grid(shorthand) => {
            target.grid_template_rows = shorthand.rows;
            target.grid_template_columns = shorthand.columns;
            target.grid_template_areas = GridTemplateAreasValue::None;
            target.grid_auto_columns = initial_grid_auto_track_list();
            target.grid_auto_rows = initial_grid_auto_track_list();
            target.grid_auto_flow = GridAutoFlowValue::Row;
            target.grid_row_start = GridLineValue::Auto;
            target.grid_row_end = GridLineValue::Auto;
            target.grid_column_start = GridLineValue::Auto;
            target.grid_column_end = GridLineValue::Auto;
        }
        PropertyValue::GridTemplateColumns(v) => target.grid_template_columns = v,
        PropertyValue::GridTemplateRows(v) => target.grid_template_rows = v,
        PropertyValue::GridTemplateAreas(v) => target.grid_template_areas = v,
        PropertyValue::GridAutoColumns(v) => target.grid_auto_columns = v,
        PropertyValue::GridAutoRows(v) => target.grid_auto_rows = v,
        PropertyValue::GridAutoFlow(v) => target.grid_auto_flow = v,
        PropertyValue::GridRowStart(v) => target.grid_row_start = v,
        PropertyValue::GridRowEnd(v) => target.grid_row_end = v,
        PropertyValue::GridColumnStart(v) => target.grid_column_start = v,
        PropertyValue::GridColumnEnd(v) => target.grid_column_end = v,
        PropertyValue::GridRow(shorthand) => {
            expand_grid_row(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::GridColumn(shorthand) => {
            expand_grid_column(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::JustifyItems(v) => target.justify_items = v,
        PropertyValue::JustifySelf(v) => target.justify_self = v,
        PropertyValue::PlaceItems(p) => expand_place_items(p, |v| apply_value(v, target)),
        PropertyValue::PlaceSelf(p) => expand_place_self(p, |v| apply_value(v, target)),
        PropertyValue::Orphans(n) => target.orphans = n,
        PropertyValue::Widows(n) => target.widows = n,
        PropertyValue::WritingMode(v) => target.writing_mode = v,
        PropertyValue::RubyPosition(v) => target.ruby_position = v,
        PropertyValue::BackgroundRepeat(v) => target.background_repeat = v,
        PropertyValue::BackgroundAttachment(v) => target.background_attachment = v,
        PropertyValue::BackgroundClip(v) => target.background_clip = v,
        PropertyValue::BackgroundOrigin(v) => target.background_origin = v,
        PropertyValue::BackgroundSize(v) => target.background_size = v,
        PropertyValue::BackgroundPosition(v) => target.background_position = v,
        PropertyValue::BackgroundImage(v) => target.background_image = v,
        PropertyValue::Background(shorthand) => {
            expand_background(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::BorderStyle(sides) => expand_border_style(sides, |v| apply_value(v, target)),
        PropertyValue::BorderWidth(sides) => expand_border_width(sides, |v| apply_value(v, target)),
        PropertyValue::BorderColor(sides) => expand_border_color(sides, |v| apply_value(v, target)),
        PropertyValue::Font(shorthand) => expand_font(&shorthand, |v| apply_value(v, target)),
        PropertyValue::ObjectFit(v) => target.object_fit = v,
        PropertyValue::ObjectPosition(v) => target.object_position = v,
        PropertyValue::Opacity(v) => target.opacity = v,
        PropertyValue::Isolation(v) => target.isolation = v,
        PropertyValue::MixBlendMode(v) => target.mix_blend_mode = v,
        PropertyValue::MaskImage(v) => target.mask_image = v,
        PropertyValue::ClipPath(v) => target.clip_path = v,
        PropertyValue::Transform(v) => target.transform = v,
        PropertyValue::Filter(v) => target.filter = v,
        PropertyValue::TableLayout(v) => target.table_layout = v,
        PropertyValue::BorderCollapse(v) => target.border_collapse = v,
        PropertyValue::BorderSpacing(v) => target.border_spacing = v,
        PropertyValue::CaptionSide(v) => target.caption_side = v,
        PropertyValue::EmptyCells(v) => target.empty_cells = v,
        // Parsed but not yet staged for elements.
        PropertyValue::TextAlignAll(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextUnderlinePosition(_) => {}
        PropertyValue::Page(value) => target.page = value,
        PropertyValue::ColumnCount(value) => target.column_count = value,
        PropertyValue::ColumnWidth(value) => target.column_width = value,
        // cov:ignore: direct unexpanded shorthand callers are defensive-only.
        PropertyValue::Columns(value) => {
            target.column_count = value.count;
            target.column_width = value.width;
        }
        // Resolved before ordinary winners reach this function.
        PropertyValue::CustomProperty(_) | PropertyValue::Deferred(_) => {}
    }
}

// cov:ignore: pure test-code relocation (no logic changed). patch-coverage's git-diff-based line classifier treats every moved
// line as newly added, and cargo-llvm-cov does not record hits for
// multi-line string-literal continuation lines inside assert!/panic!
// messages even though the containing statement executes in a
// passing test. Verified against every flagged line in this move:
// all are string-literal fragments or trivial format-arg
// expressions inside already-passing tests, none of them
// production code.
#[cfg(test)]
mod tests;
