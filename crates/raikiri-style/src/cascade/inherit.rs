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

        let custom_properties = cascaded
            .custom_candidates(id)
            .map(|candidates| resolve_custom_properties(&parent_custom_properties, candidates))
            .unwrap_or_else(|| parent_custom_properties.clone());

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
                let pseudo_custom_properties = custom_candidates
                    .map(|candidates| resolve_custom_properties(&custom_properties, candidates))
                    .unwrap_or_else(|| custom_properties.clone());

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
/// length を運ぶ property は **specified 表現のまま**格納する — 絶対化は
/// [`SpecifiedValues::finalize`] (phase 2 + phase 3) の責務であり、本関数の中で
/// 行うことは意図的に禁じられている (`padding: 2em` の
/// 基準となる font-size は**その node の全 winner を適用し終える**まで確定せず、
/// 本関数は winner 1 つ分しか見ていないため)。
///
/// 例外は `font-weight` の `bolder` / `lighter` と `font-size` の `larger` /
/// `smaller` — どちらも**継承元**の computed 値だけで解ける (自 node の他
/// winner に依存しない) ため、ここで絶対値に落とす。詳細は該当 arm の comment
/// を参照。
///
/// `pub(crate)` は他 module の doc からの intra-doc link のため — private 化で gate が red (規約 3)。
pub(crate) fn apply_value(value: PropertyValue, target: &mut SpecifiedValues) {
    match value {
        PropertyValue::Color(c) => target.color = c,
        // CSS Backgrounds 3 §2.2。sibling `Color` と対称的な
        // 単純代入 (non-inherited、per-node で cascade winner を直接反映)。
        PropertyValue::BackgroundColor(c) => target.background_color = c,
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        // CSS Fonts 4 §2.5 `<relative-size>` (`larger` / `smaller`)。
        // `font-weight` の `bolder` / `lighter` arm
        // (次項) と同型の read-modify-write だが、継承値の出所は D5 invariant
        // (`font_weight: u16`) とは少し違う形で成立する:
        //
        // - `target.font_size` は `SpecifiedValues::inherit_from(parent)` で
        //   `lift_font_size(parent.font_size)` = 常に `Length::Px(親の px)`
        //   に seed される (root では `SpecifiedValues::initial()` が同じく
        //   `Length::Px(INITIAL_FONT_SIZE_PX)`)。したがって本 arm が
        //   `target.font_size` を**上書きする前に読む**限り、変数の中身は
        //   単位に関わらず「親 (または root の initial) の computed
        //   font-size」の px 表現である。
        // - `font_size` の他の値 (`em` / `rem` / `%`) は絶対化を
        //   `SpecifiedValues::finalize` (phase 2) に **意図的に遅延**する
        //   (本関数冒頭の doc 参照) が、
        //   `larger` / `smaller` は基準が「親の computed font-size」のみで
        //   自 node の他 winner に依存しないため、`font-weight` と同じく
        //   ここ (phase 1) で解決してよい。解決結果は `Length::Px` — 通常の
        //   author 指定 px 値と区別が付かなくなり、phase 2 (`resolve_font_size`
        //   の `Px` arm は identity) を通しても二重適用にならない。
        // - 全 `Length` variant を OR-pattern で受ける下の抽出は
        //   「実際には常に `Px`」を panic-free に表現したもの — panic surface を
        //   作らない方針 (`Margin` shorthand fall-through arm と同じ
        //   理由) により `unreachable!` は採らない。
        //
        // `FontSize` と同じ `PropertyKey` を共有するため (`PropertyValue::key()`
        // 参照) `pick_winners` の slot は 1 つ — 本 arm と直上の `FontSize` arm が
        // 同一 node で両方走ることはない。
        PropertyValue::FontSizeRelative(rel) => {
            // cov:ignore: per the doc comment above, `target.font_size` is
            // always `Length::Px` at this point (post phase-2 resolution) —
            // only the `Px` arm is ever exercised. The other 15 arms exist
            // to make this extraction panic-free (no `unreachable!`), not
            // because any test constructs a non-Px font_size here.
            //
            // Re-examined after `font-size: 1lh` / `1rlh` became
            // parse-accepted, removing the *previous* reason
            // this was unreachable — that `parse_font_size` dropped them at
            // parse time. The `Lh`/`Rlh` arms remain unreachable, but for a
            // different, still-true reason: `FontSize` and `FontSizeRelative`
            // share one `PropertyKey::FontSize` slot per node
            // (`PropertyValue::key()`), so at most one of them is the winner
            // applied to any given node. Whenever *this* arm runs for a
            // node, the `FontSize` arm (above) did *not* also run for that
            // same node — so `target.font_size` was never overwritten by
            // this node's own declaration and still holds the seed from
            // `SpecifiedValues::inherit_from`, which is always
            // `lift_font_size(parent.font_size) == Length::Px(_)`
            // (`lift_font_size` always returns `Px`, regardless of what unit
            // the parent's own font-size declaration used, since the parent
            // has already been absolutized to a `ComputedLength` by the time
            // this node inherits from it). This invariant does not depend on
            // which `Length` units `parse_font_size` accepts.
            let inherited_px = match target.font_size {
                Length::Px(v)
                | Length::Em(v)
                | Length::Rem(v)
                | Length::Percent(v)
                | Length::Pt(v)
                | Length::Ex(v)
                | Length::Rex(v)
                | Length::Ch(v)
                | Length::Rch(v)
                | Length::Ic(v)
                | Length::Ric(v)
                | Length::Cm(v)
                | Length::Mm(v)
                | Length::Q(v)
                | Length::In(v)
                | Length::Pc(v)
                | Length::Lh(v)
                | Length::Rlh(v) => v,
            };
            target.font_size = Length::Px(resolve_relative_font_size(rel, inherited_px));
        }
        // CSS Fonts 4 §2.2。specified
        // value は `FontWeightValue` (relative keyword を保持)、computed value
        // は resolve 済み `f32` (`u16` から格上げ済み) —
        // `bolder` / `lighter` はここで絶対値に落とす。
        //
        // 継承値の出所: `target` は直前に `SpecifiedValues::inherit_from(parent)`
        // で seed されており (`resolve_inheritance` 参照)、`font_weight` は
        // inherited property なので **この時点の `target.font_weight` は親の
        // computed font-weight そのもの**。`SpecifiedValues` が
        // `font_weight: f32` を「既に computed-equivalent」として持つのはこの
        // invariant のため — `SpecifiedValues::initial()` から seed する実装に
        // 変えると `bolder` が常に 400 起点になり、compile error にも既存 test の
        // 失敗にもならずに壊れる (D5 invariant)。
        // さらに `pick_winners` は
        // `PropertyKey` ごとに slot を 1 つだけ埋めるため `FontWeight` arm が同一
        // node で 2 回走ることはなく、winner の適用順にも依存しない
        // (`winner_does_not_leak_into_next_sibling` test がこの "1 回だけ" を check
        // する — 二重適用は 400 → 700 → 900 と複合するので観測可能)。
        // この 2 つが relative-weight resolution の正しさを支える invariant。
        //
        // なお本 arm は `apply_value` 中の read-modify-write の 1 つ (`FontSizeRelative`
        // arm が 2 つ目に加わった。それ以外はすべて冪等な
        // 単純代入)。`Padding` / `Margin` / `Border` shorthand fall-through arm
        // のような二重適用経路を font-weight に足すと `bolder` が 400 → 700 → 900
        // と複合するため、上記 2 invariant を崩す変更は不可。`FontSizeRelative` も
        // 同じ理由で二重適用経路を持たない (`FontSize` と同一 `PropertyKey`
        // を共有し slot は 1 つ、詳細は該当 arm の comment)。
        //
        // **契約**: 継承元依存の解決を持つ property を新しく
        // 追加するときは、本 arm だけでなく sibling の
        // `resolve_against_inherited` にも arm を足すこと — そちらは
        // `PropertyValue` を返す形で同じ解決を提供し、
        // `crate::page::cascade_page` (`apply_value` を通らない第 2 の public
        // entry point) が使う。両者とも wildcard 無しの exhaustive match なので
        // variant 追加時は compiler が 2 経路を数え上げさせる。
        //
        // **例外**: `text-align: match-parent` は
        // `resolve_against_inherited` に arm があるが、**本 arm (`apply_value`)
        // には無い** — 下の `TextAlign` arm のコメント参照。この property の
        // 解決は `self.font_weight` のような自 field の read-modify-write では
        // 済まない (**他 property `direction` の親の値**を要する) ため、
        // 「本 arm と sibling の 2 経路だけ数え上げればよい」という上記契約の
        // 前提が破れる — 実際には 3 箇所目 (`crate::specified::SpecifiedValues::
        // finalize` / `finalize_as_root`) が element 経路の解決を担う。
        PropertyValue::FontWeight(fw) => {
            let inherited = target.font_weight;
            target.font_weight = resolve_relative_weight(fw, inherited);
        }
        // CSS Inline 3 §5.1: line-height は inherited、cascade winner が
        // raw value (Normal / Number / Length) を保持。number-vs-length
        // distinction は下流 (paint) の resolve context で意味を持つ。
        PropertyValue::LineHeight(lh) => target.line_height = lh,
        PropertyValue::Display(d) => target.display = d,
        PropertyValue::ListStyleType(v) => target.list_style_type = v,
        PropertyValue::ListStyleImage(v) => target.list_style_image = v,
        PropertyValue::ListStylePosition(v) => target.list_style_position = v,
        // counter-* は将来の GCPM (paged media generated content) 対応に
        // 向けた足場 — parse 結果をそのまま computed value に格納。counter
        // tree の実際の resolve は将来の本実装で行う。
        PropertyValue::CounterReset(v) => target.counter_reset = v,
        PropertyValue::CounterResetInherit => {
            target.counter_reset = crate::property::empty_counter_entries();
        }
        PropertyValue::CounterIncrement(v) => target.counter_increment = v,
        PropertyValue::CounterSet(v) => target.counter_set = v,
        // content は将来の GCPM directive-emit の static-side 実装。
        // 下流 (raikiri-dom) runtime resolve が counter()/string()/target-*() の
        // 実値を組み立てる際に本 field を参照。
        PropertyValue::Content(v) => target.content = v,
        // string-set は将来の GCPM static-side 実装の一部 (CSS GCPM 3 §1.1.1)。
        // Named-string runtime resolve は下流 (raikiri-dom) 責務。
        PropertyValue::StringSet(v) => target.string_set = v,
        // position は将来の GCPM static-side 実装の一部 (CSS GCPM 3 §1.2.1)。
        // - `Static` は no-op: `inherit_from` が running_templates を空で初期化
        //   するため、`position: static` が cascade winner のとき running_templates
        //   は空のままで正しい (先行 running(hdr) を上書きして
        //   template 登録を suppress する用途)。
        // - `Running(name)` は 1-item seed を push。per-node で常に 0/1 要素
        //   (position は spec 上 単一値)、per-document 集約は下流 (raikiri-dom)
        //   の 2-tier キャッシュ static side 責務 (design doc §7.3)。
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
        // text-align (CSS Text 3 §6.1) の初期実装。
        // inherited property のため cascade winner が無い child は inherit_from で
        // 親値を引き継ぐ (color / font_family / font_size / font_weight と同じ
        // handling)。TextAlign は Copy、by-value 代入で十分。
        //
        // **`match-parent` はここでは解決しない** — この
        // 単純代入は他 arm と同じく素朴なコピーのままにしてある。解決は
        // `crate::specified::SpecifiedValues::finalize` /
        // `finalize_as_root` が全 winner 適用**後**に、明示的な親
        // `ComputedValues` を受け取って行う。理由: 本 arm の中で
        // `target.direction` (= 親から継承した direction) を読んで解決しようと
        // すると、**同一 node が `direction` winner も持つ場合**にその適用順序
        // (`crate::property::PropertyKey` の宣言順) 次第で親ではなく
        // **自分の** direction を読んでしまう —
        // `resolve_inheritance` が保証する「winner の適用順に依存しない」
        // invariant への違反になる。詳細は
        // `crate::property::resolve_text_align_match_parent` の doc。
        PropertyValue::TextAlign(t) => target.text_align = t,
        // CSS Text 3 §8.2.1: inherited keyword, simple by-value assignment.
        PropertyValue::HangingPunctuation(v) => target.hanging_punctuation = v,
        // CSS Text 3 §6.2 text-justify — **inherited** keyword、単純代入。
        PropertyValue::TextJustify(v) => target.text_justify = v,
        // CSS Text 3 §6.1 text-align-last — **inherited** keyword、単純代入。
        // `auto` の解決は consumer 側 (raikiri-dom realign)。
        PropertyValue::TextAlignLast(v) => target.text_align_last = v,
        // CSS Text 3 §8.1 text-indent — **inherited**. `Length` is `Copy`,
        // by-value assignment suffices (sibling `TextAlign`/`Direction`
        // pattern). Absolutization (`em`/`rem`/`%` etc.) happens later in
        // `SpecifiedValues::finalize` / `finalize_as_root`, mirroring
        // `padding`'s phase-3 handling — the only difference from `padding`
        // is that this field is inherited, so `SpecifiedValues::inherit_from`
        // (not this function) is what seeds a child with no winner of its
        // own.
        // CSS Text 3 §8.1 text-indent — **inherited**. The length goes to
        // `text_indent`; the flags go to their own staging fields so the
        // specified/computed layers keep the full grammar.
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
        // direction は CSS Writing Modes 4 §2.1。
        // inherited property、computed value = specified value (相対解決なし) —
        // text-align と同じく単純代入で十分。
        PropertyValue::Direction(d) => target.direction = d,
        // CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical>
        // padding physical longhand。
        // 4 side を独立に上書き。shorthand `PropertyValue::Padding` は
        // `crate::rule::expand_shorthand_into` により parse 出口と element cascade 入口
        // (`collect_cascaded`) の両方で 4 longhand に展開されるため、cascade 段に
        // 届く declaration は per-side longhand
        // のみ = shorthand/longhand の cross-key dependency が消え、winner の
        // 適用順に依存しない per-key determinism が成立する
        // (margin の parse-time expansion model と同じ設計に migrate 済み)。
        PropertyValue::PaddingTop(v) => target.padding.top = v,
        PropertyValue::PaddingRight(v) => target.padding.right = v,
        PropertyValue::PaddingBottom(v) => target.padding.bottom = v,
        PropertyValue::PaddingLeft(v) => target.padding.left = v,
        // CSS Box 3 §4.2 <https://www.w3.org/TR/css-box-3/#padding-shorthand>
        // padding shorthand fall-through (normal flow では展開済み)。
        // sibling `PropertyValue::Margin` arm と同じく **safety net ではない** —
        // 到達すれば 4 longhand winner を破壊し spec と食い違う。
        // Sides<Length>: Copy のため move で `target.padding` に代入。
        PropertyValue::Padding(sides) => target.padding = sides,
        // `padding-inline`/`padding-block` shorthand fall-through — sibling
        // `PropertyValue::Padding` arm と同じく **safety net ではない**。
        // cascade 経路では unreachable (`crate::rule::expand_shorthand_into`
        // が `padding-left`/`padding-right` (block なら `padding-top`/
        // `padding-bottom`) の 2 longhand に展開する)。物理写像 (inline axis
        // は `direction: ltr` 仮定の近似、block axis は厳密) の rationale は
        // `crate::property::PropertyValue::PaddingInline` doc 参照。挙動は
        // `apply_value_direct_padding_inline_shorthand_fall_through` /
        // `apply_value_direct_padding_block_shorthand_fall_through` test が
        // 直接叩いて check (`Margin` arm 上の
        // `apply_value_direct_margin_shorthand_fall_through` precedent)。
        PropertyValue::PaddingInline(pair) => {
            target.padding.left = pair.start;
            target.padding.right = pair.end;
        }
        PropertyValue::PaddingBlock(pair) => {
            target.padding.top = pair.start;
            target.padding.bottom = pair.end;
        }
        // 4 longhand margin sides (CSS Box 3 §3.1)。
        // shorthand `PropertyValue::Margin` は `crate::rule::expand_shorthand_into`
        // により parse 出口と element cascade 入口の両方で 4 longhand に展開されるため、
        // cascade 段に届く declaration
        // は per-side longhand のみ = winner の適用順に依存しない per-key
        // determinism が成立する (詳細は `crate::rule::expand_shorthand_into` doc)。
        // Page-only inherit markers are never produced by the element parser;
        // keep the staging path panic-free if an internal caller supplies one.
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
        // Parsed shorthands normally arrive as longhands. If a shorthand is
        // supplied directly, retain its value without panicking; normal
        // cascade entry points expand it before this stage.
        PropertyValue::Margin(sides) => target.margin = sides,
        // `margin-inline`/`margin-block` shorthand fall-through — sibling
        // `PropertyValue::Margin` arm と同じく **safety net ではない**。
        // cascade 経路では unreachable (`crate::rule::expand_shorthand_into`
        // が `margin-left`/`margin-right` (block なら `margin-top`/
        // `margin-bottom`) の 2 longhand に展開する)。物理写像の rationale は
        // `crate::property::PropertyValue::MarginInline` doc 参照。挙動は
        // `apply_value_direct_margin_inline_shorthand_fall_through` /
        // `apply_value_direct_margin_block_shorthand_fall_through` test が
        // 直接叩いて pin。
        PropertyValue::MarginInline(pair) => {
            target.margin.left = pair.start;
            target.margin.right = pair.end;
        }
        PropertyValue::MarginBlock(pair) => {
            target.margin.top = pair.start;
            target.margin.bottom = pair.end;
        }
        // CSS Backgrounds 3 §3.3/§3.2/§3.1 border physical longhand。
        // 4 side × 3 sub-property の 12 arm。shorthand
        // `PropertyValue::Border` は `crate::rule::expand_shorthand_into` により
        // parse 出口と element cascade 入口の両方で 12 longhand に展開されるため、
        // cascade 段に届く declaration
        // は per-side / per-sub-property longhand のみ = winner の適用順に
        // 依存しない per-key determinism が成立する (margin / padding precedent
        // 踏襲)。
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
        // `border` shorthand fall-through。sibling `PropertyValue::Margin` arm と
        // 同じく **safety net ではない** — 到達すれば 12 longhand winner を一括で
        // 破壊し spec と食い違う。cascade 経路では unreachable
        // (`expand_shorthand_into` が 12 longhand に展開する)。詳細な framing と
        // その unreachability の compile-time 強制は
        // `Margin` arm の comment 参照。
        PropertyValue::Border(sides) => target.border = sides,
        // CSS Sizing 3 §3.1.1 width。single-value property、
        // `LengthOrAuto` は Copy shape (Length variant は Copy)。sibling
        // `PropertyValue::TextAlign` と対称的な単純代入 (non-inherited、cascade
        // winner を直接反映)。`auto` は下流 layout の automatic size calculation
        // (CSS Sizing 3 §5) で解決される — margin `auto` の余白分配とは別意味。
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
        // CSS Sizing 3 §3.1.1 height。sibling `Width` /
        // `Padding*` / `Margin*` と同じ per-node winner 直接代入 (non-inherited、
        // `LengthOrAuto` は Copy)。resolve (`Percent` / `Auto` の実 layout 高さ
        // 計算) は下流責務。
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
        // CSS Sizing 3 §3.3 box-sizing。non-inherited、
        // cascade winner が specified keyword をそのまま computed value に反映。
        // BoxSizing は Copy、by-value 代入で十分。
        PropertyValue::BoxSizing(bs) => target.box_sizing = bs,
        // CSS Overflow 3 §3.1 overflow-x/overflow-y physical longhand。
        // non-inherited、per-axis winner を staging の
        // `overflow.x`/`overflow.y` へ直接代入。cross-axis の computed-value
        // coupling (`resolve_overflow`) はここでは**適用しない** —
        // `target.overflow` は winner 適用の途中経過であり、まだ他方の axis の
        // 最終 winner を反映し終えていない可能性がある。coupling は全 winner
        // 適用後の phase 3 (`SpecifiedValues::finalize` → `absolutize_with`)
        // でのみ解決する (border style→width gating と同じ順序、
        // `resolve_overflow` doc 参照)。
        PropertyValue::OverflowX(v) => target.overflow.x = v,
        PropertyValue::OverflowY(v) => target.overflow.y = v,
        // `overflow` shorthand fall-through。sibling `PropertyValue::Padding`
        // arm と同じく **safety net ではない** — 到達すれば 2 longhand winner
        // を一括で破壊し spec と食い違う。cascade 経路では unreachable
        // (`expand_shorthand_into` が 2 longhand に展開する)。詳細な framing
        // とその unreachability の compile-time 強制は `Margin` arm の
        // comment 参照。
        PropertyValue::Overflow(pair) => target.overflow = pair,
        // CSS Text Decoration Module Level 3 §2.1-§2.3。3 longhand とも
        // non-inherited、cascade winner が specified keyword/color をそのまま
        // computed value に反映。いずれも Copy、by-value 代入で十分
        // (`BoxSizing` arm と同型)。
        PropertyValue::TextDecorationLine(v) => target.text_decoration_line = v,
        PropertyValue::TextDecorationStyle(v) => target.text_decoration_style = v,
        PropertyValue::TextDecorationColor(v) => target.text_decoration_color = v,
        // CSS Text Decoration 4 §2.9.1: non-inherited, but font-relative
        // lengths remain in staging until `SpecifiedValues::finalize`.
        PropertyValue::TextDecorationInset(v) => target.text_decoration_inset = v,
        // CSS Text Decoration 4 §2.8: inherited; fixed lengths remain in
        // staging until `SpecifiedValues::finalize` resolves them.
        PropertyValue::TextUnderlineOffset(v) => target.text_underline_offset = v,
        // `text-decoration` shorthand fall-through。sibling `PropertyValue::Margin`
        // arm と同じく **safety net ではない** — 到達すれば 3 longhand winner を
        // 一括で破壊し spec と食い違う。cascade 経路では unreachable
        // (`expand_shorthand_into` が 3 longhand に展開する)。詳細な framing と
        // その unreachability の compile-time 強制は `Margin` arm の comment
        // 参照。
        PropertyValue::TextDecoration(shorthand) => {
            target.text_decoration_line = shorthand.line;
            target.text_decoration_style = shorthand.style;
            target.text_decoration_color = shorthand.color;
            // `thickness` has no staging field (parsing-only,
            // `PropertyValue::TextDecorationThickness` doc) — nothing to
            // write here. Same unreachability contract as the 3 staged
            // longhands above.
        }
        // CSS 2.1 §10.8.1 vertical-align。non-inherited、cascade winner を
        // このまま staging (`SpecifiedValues`) へ書き込む — `<length>`
        // variant の絶対化 (`em`/`rem` 等) は phase 3
        // (`SpecifiedValues::absolutize_with`'s `resolve_vertical_align`
        // call) の役目でここでは行わない (`FlexBasis` arm と同型)。
        // `VerticalAlign` は Copy、by-value 代入で十分 (`BoxSizing` /
        // `TextDecorationLine` arm と同型)。
        PropertyValue::VerticalAlign(va) => target.vertical_align = va,
        // CSS Fonts 4 §2.4。
        // inherited property のため cascade winner が無い child は
        // inherit_from で親値を引き継ぐ (`Direction` arm と同じ handling)。
        // `FontStyle` は Copy、by-value 代入で十分。
        PropertyValue::FontStyle(fs) => target.font_style = fs,
        // CSS Text Module Level 3 §2.1。
        // inherited property のため cascade winner が無い child は
        // inherit_from で親値を引き継ぐ (`FontStyle` arm と同じ handling)。
        // `TextTransform` は Copy、by-value 代入で十分。
        PropertyValue::TextTransform(tt) => target.text_transform = tt,
        // CSS Display 3 §4。
        // inherited property のため cascade winner が無い child は
        // inherit_from で親値を引き継ぐ (`Direction` / `FontStyle` arm と同じ
        // handling)。`Visibility` は Copy、by-value 代入で十分。
        PropertyValue::Visibility(v) => target.visibility = v,
        // CSS2 §9.9.1 z-index。non-inherited、cascade winner が specified
        // value をそのまま computed value に反映。`ZIndexValue` は Copy、
        // by-value 代入で十分 (`BoxSizing` / `VerticalAlign` arm と同型)。
        PropertyValue::ZIndex(z) => target.z_index = z,
        // word-break は CSS Text 3 §5.1。inherited property、
        // computed value = specified value (相対解決なし) — sibling
        // `FontStyle` と同じく単純代入で十分。
        PropertyValue::WordBreak(wb) => target.word_break = wb,
        // line-break is inherited and its keyword is already computed-equivalent.
        PropertyValue::LineBreak(lb) => target.line_break = lb,
        // overflow-wrap (legacy alias 名 word-wrap も同じ variant/field に
        // 落ちる、`OverflowWrap` doc 参照) は CSS Text 3 §5.4。
        // inherited property、computed value = specified value
        // (相対解決なし) — sibling `WordBreak` と同じく単純代入で十分。
        PropertyValue::OverflowWrap(ow) => target.overflow_wrap = ow,
        // CSS Text 3 §7.2。specified 表現 (`LengthOrNormal`) のまま格納 —
        // 絶対化は phase 3 (`SpecifiedValues::finalize` / `absolutize_with`)
        // に委ねる (関数冒頭の doc「length を運ぶ property は specified 表現の
        // まま格納する」節)。inherited property のため cascade winner が無い
        // child は inherit_from で親値を引き継ぐ (`FontStyle` arm と同じ
        // handling)。`LengthOrNormal` は Copy、by-value 代入で十分。
        PropertyValue::LetterSpacing(ls) => {
            target.letter_spacing = ls;
            target.letter_spacing_ch_factor = match ls {
                LengthOrNormal::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                _ => None,
            };
        }
        // CSS Text 3 §7.1。直上の LetterSpacing arm と同型。
        PropertyValue::WordSpacing(ws) => {
            target.word_spacing = ws;
            target.word_spacing_ch_factor = match ws {
                LengthOrNormal::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                _ => None,
            };
        }
        // CSS Text Module Level 3 §4.2。specified 表現 (`TabSize`) のまま
        // 格納 — `<length>` 側の絶対化は phase 3 に委ねる (`LetterSpacing`
        // arm と同じ handling)。inherited property のため cascade winner が
        // 無い child は inherit_from で親値を引き継ぐ。`TabSize` は Copy、
        // by-value 代入で十分。
        PropertyValue::TabSize(ts) => target.tab_size = ts,
        // CSS Fragmentation Module Level 3 §3.1 break-before / break-after
        // (legacy shorthand page-break-before / page-break-after も同じ
        // variant/field に落ちる、`BreakBetween` doc 参照)。non-inherited、
        // cascade winner が specified keyword をそのまま computed value に
        // 反映。`BreakBetween` は Copy、by-value 代入で十分 (`ZIndex` arm と
        // 同型)。
        PropertyValue::BreakBefore(bb) => target.break_before = bb,
        PropertyValue::BreakAfter(bb) => target.break_after = bb,
        // CSS Fragmentation Module Level 3 §3.2 break-inside (legacy
        // shorthand page-break-inside も同じ field に落ちる、`BreakInside`
        // doc 参照)。直上の BreakBefore/BreakAfter arm と同型。
        PropertyValue::BreakInside(bi) => target.break_inside = bi,
        // CSS2 §9.5.1 float。non-inherited、cascade winner が specified
        // value をそのまま格納する。`display` への §9.7 の強制変換は
        // ここでは行わない — phase 3 (`SpecifiedValues::absolutize_with`)
        // が両 field 確定後にまとめて解決する (`resolve_display_for_float`
        // doc)。`FloatValue` は Copy、by-value 代入で十分 (`ZIndexValue`
        // arm と同型)。
        PropertyValue::Float(f) => target.float = f,
        // CSS2 §9.5.2 clear。non-inherited、cascade winner が specified
        // value をそのまま格納する — sibling `Float` arm と同じく単純代入で
        // 十分。
        PropertyValue::Clear(c) => target.clear = c,
        // CSS Text 3 §3 white-space。inherited property、computed value =
        // specified value (相対解決なし) — sibling `WordBreak` と同じく
        // 単純代入で十分。
        PropertyValue::WhiteSpace(ws) => target.white_space = ws,
        // CSS Text 4 §5 text-wrap (subset). Inherited keyword, computed value =
        // specified keyword — simple assignment like `WhiteSpace` above.
        PropertyValue::TextWrap(v) => target.text_wrap = v,
        // CSS Text 3 §5.3 hyphens。inherited property、computed value =
        // specified keyword (相対解決なし、`Hyphens` doc 参照) — sibling
        // `WordBreak` と同じく単純代入で十分。
        PropertyValue::Hyphens(h) => target.hyphens = h,
        // CSS Flexible Box Layout Module Level 1 §5.1/§5.2。non-inherited、
        // computed value = specified keyword (相対解決なし) — 単純代入で十分。
        PropertyValue::FlexDirection(fd) => target.flex_direction = fd,
        PropertyValue::FlexWrap(fw) => target.flex_wrap = fw,
        // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2。
        // non-inherited、computed value = specified number — 単純代入で十分。
        PropertyValue::FlexGrow(g) => target.flex_grow = g,
        PropertyValue::FlexShrink(s) => target.flex_shrink = s,
        // CSS Flexible Box Layout Module Level 1 §7.2.3。specified 表現
        // (`FlexBasisValue`) のまま格納 — 絶対化は phase 3 に委ねる
        // (`LetterSpacing` arm と同じ handling)。non-inherited。
        PropertyValue::FlexBasis(fb) => target.flex_basis = fb,
        // `flex` shorthand fall-through — 通常は `expand_shorthand_into` が
        // 3 longhand に展開済みのため cascade 経路には到達しない
        // (`Margin`/`Padding`/`Border` shorthand fall-through と同じ
        // "safety net ではない" 位置付け、`PropertyKey::Padding` doc 参照)。
        // cov:ignore: exercised by apply_value_direct_flex_shorthand_fall_through
        // (a passing test in this same file) but cargo-llvm-cov does not
        // attribute hits to this arm's lines within apply_value's large
        // match statement.
        PropertyValue::Flex(f) => {
            target.flex_grow = f.grow;
            target.flex_shrink = f.shrink;
            target.flex_basis = f.basis;
        }
        // `flex-flow` shorthand fall-through (`Flex` arm と同じ位置付け)。
        PropertyValue::FlexFlow(f) => {
            target.flex_direction = f.direction;
            target.flex_wrap = f.wrap;
        }
        // CSS Flexible Box Layout Module Level 1 §4.2。non-inherited、
        // computed value = specified integer — 単純代入で十分。
        PropertyValue::Order(o) => target.order = o,
        // CSS Box Alignment Module Level 3 §5.1 (justify-content /
        // align-content) / §7.2 (align-items) / §6.2 (align-self)。
        // non-inherited、computed value = specified keyword(s) —
        // 単純代入で十分。
        PropertyValue::JustifyContent(jc) => target.justify_content = jc,
        PropertyValue::AlignContent(ac) => target.align_content = ac,
        PropertyValue::AlignItems(ai) => target.align_items = ai,
        PropertyValue::AlignSelf(as_) => target.align_self = as_,
        // CSS Box Alignment Module Level 3 §8.1。specified 表現
        // (`LengthOrNormal`) のまま格納 — 絶対化は phase 3 に委ねる。
        // non-inherited。
        PropertyValue::RowGap(rg) => target.row_gap = rg,
        PropertyValue::ColumnGap(cg) => target.column_gap = cg,
        // `gap` shorthand fall-through (`Flex` arm と同じ位置付け)。
        PropertyValue::Gap(g) => {
            target.row_gap = g.row;
            target.column_gap = g.column;
        }
        // `place-content` shorthand fall-through (`Flex` arm と同じ位置付け)。
        PropertyValue::PlaceContent(p) => {
            target.align_content = p.align;
            target.justify_content = p.justify;
        }
        // CSS Fonts Module Level 3 §6.6。
        // inherited property のため cascade winner が無い child は
        // inherit_from で親値を引き継ぐ (`FontStyle` arm と同じ handling)。
        // `FontVariantCaps` は Copy、by-value 代入で十分。
        PropertyValue::FontVariantCaps(fvc) => target.font_variant_caps = fvc,
        // CSS Content 3 §2.4.1: quotes は将来の GCPM (paged media generated
        // content) 対応に向けた足場 — parse 結果をそのまま computed value に
        // 格納する (`CounterReset`/`Content` arm と同じ位置付け、
        // `ComputedValues::quotes` doc 参照)。nesting depth → 実際の
        // 引用符文字列への解決は下流 (raikiri-dom) 責務。
        PropertyValue::Quotes(v) => {
            target.quotes_auto = false;
            target.quotes = v;
        }
        // CSS Text Decoration Module Level 3 §4。specified 表現
        // (`Arc<Vec<TextShadowItem>>`) のまま格納 — 絶対化 (各 item の length
        // 3 本) は phase 3 (`SpecifiedValues::finalize` / `absolutize_with`)
        // に委ねる (`LetterSpacing`/`FlexBasis` arm と同じ handling)。
        // inherited property のため、cascade winner が無い child は
        // `inherit_from` で親値 (lift 済み) を引き継ぐ。`Arc` は Clone が
        // bump のみなので by-value 代入で十分。
        PropertyValue::TextShadow(shadows) => target.text_shadow = shadows,
        PropertyValue::BorderRadius(v) => target.border_radius = v,
        PropertyValue::BorderRadiusTopLeft(v) => target.border_radius.top_left = v,
        PropertyValue::BorderRadiusTopRight(v) => target.border_radius.top_right = v,
        PropertyValue::BorderRadiusBottomRight(v) => target.border_radius.bottom_right = v,
        PropertyValue::BorderRadiusBottomLeft(v) => target.border_radius.bottom_left = v,
        PropertyValue::BoxShadow(shadows) => target.box_shadow = shadows,
        PropertyValue::Outline(v) => target.outline = v,
        PropertyValue::OutlineWidth(v) => target.outline.width = v,
        PropertyValue::OutlineStyle(v) => target.outline.style = v,
        PropertyValue::OutlineColor(v) => target.outline.color = v,
        PropertyValue::OutlineOffset(v) => target.outline_offset = v,
        // CSS Grid Layout Module Level 1 §8.4 `grid-area` shorthand.
        PropertyValue::GridArea(area) => {
            target.grid_row_start = area.row_start;
            target.grid_column_start = area.column_start;
            target.grid_row_end = area.row_end;
            target.grid_column_end = area.column_end;
        }
        // CSS Grid Layout Module Level 1 `grid` shorthand. Its supported
        // explicit-track form also resets the other grid sub-properties.
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
        // CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/§7.7/§8.3。
        // non-inherited、specified 表現のまま格納 — 絶対化は phase 3 に
        // 委ねる (`LetterSpacing`/`FlexBasis` arm と同じ handling)。
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
        // `grid-row` / `grid-column` shorthand fall-through — 通常は
        // `expand_shorthand_into` が 2 longhand に展開済みのため cascade
        // 経路には到達しない (`Flex` arm と同じ位置付け)。
        PropertyValue::GridRow(shorthand) => {
            target.grid_row_start = shorthand.start;
            target.grid_row_end = shorthand.end;
        }
        PropertyValue::GridColumn(shorthand) => {
            target.grid_column_start = shorthand.start;
            target.grid_column_end = shorthand.end;
        }
        // CSS Box Alignment Module Level 3 §7.1 (justify-items) / §6.1
        // (justify-self)。non-inherited、computed value = specified
        // keyword(s) — 単純代入で十分。
        PropertyValue::JustifyItems(v) => target.justify_items = v,
        PropertyValue::JustifySelf(v) => target.justify_self = v,
        // `place-items` / `place-self` shorthand fall-through (`Flex` arm
        // と同じ位置付け)。
        PropertyValue::PlaceItems(p) => {
            target.align_items = p.align;
            target.justify_items = p.justify;
        }
        PropertyValue::PlaceSelf(p) => {
            target.align_self = p.align;
            target.justify_self = p.justify;
        }
        // CSS Fragmentation Module Level 3 §3.3. inherited、computed value =
        // specified integer — simple assignment (`FlexGrow`/`FlexShrink` arm
        // と同じ shape、length を運ばないため絶対化不要)。
        PropertyValue::Orphans(n) => target.orphans = n,
        PropertyValue::Widows(n) => target.widows = n,
        // CSS Writing Modes 4 §3.2. inherited, simple assignment — the
        // `HorizontalTb` collapse for the 4 non-horizontal keywords does
        // *not* happen here (`resolve_writing_mode` runs later, in
        // `SpecifiedValues::absolutize_with`, same "staging isn't resolved
        // yet" split `TextAlign`'s `match-parent` uses). `target.writing_mode`
        // is a `SpecifiedValues` field, not `ComputedValues` — see
        // `WritingMode` doc's Non-goal section.
        PropertyValue::WritingMode(v) => target.writing_mode = v,
        PropertyValue::RubyPosition(v) => target.ruby_position = v,
        // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8. non-inherited,
        // computed value = specified keyword(s) — simple assignment, no
        // length payload (`BackgroundColor`/`Orphans` arm と同じ shape)。
        PropertyValue::BackgroundRepeat(v) => target.background_repeat = v,
        PropertyValue::BackgroundAttachment(v) => target.background_attachment = v,
        PropertyValue::BackgroundClip(v) => target.background_clip = v,
        PropertyValue::BackgroundOrigin(v) => target.background_origin = v,
        // CSS Backgrounds and Borders 3 §2.9/§2.6. non-inherited、
        // `<length-percentage>` を含むため specified 表現のまま格納 —
        // 絶対化は phase 3 (`SpecifiedValues::absolutize_with`) に委ねる
        // (`Width`/`Padding` arm と同じ shape)。
        PropertyValue::BackgroundSize(v) => target.background_size = v,
        PropertyValue::BackgroundPosition(v) => target.background_position = v,
        // CSS Backgrounds and Borders 3 §2.3. non-inherited, computed value
        // = specified value — simple assignment, no length payload
        // (`BackgroundRepeat` arm と同じ shape)。
        PropertyValue::BackgroundImage(v) => target.background_image = v,
        // `background` shorthand fall-through (see `Padding`/`Flex` arm docs
        // above for the "not a safety net" framing) — unreachable in
        // practice, `expand_shorthand_into` expands it to the 8
        // `BackgroundColor`/`BackgroundImage`/`BackgroundRepeat`/
        // `BackgroundAttachment`/`BackgroundPosition`/`BackgroundSize`/
        // `BackgroundClip`/`BackgroundOrigin` longhands before `apply_value`
        // ever sees it.
        PropertyValue::Background(shorthand) => {
            target.background_color = shorthand.color;
            target.background_image = shorthand.image;
            target.background_repeat = shorthand.repeat;
            target.background_attachment = shorthand.attachment;
            target.background_position = shorthand.position;
            target.background_size = shorthand.size;
            target.background_clip = shorthand.clip;
            target.background_origin = shorthand.origin;
        }
        // `font` shorthand fall-through (see `Padding`/`Flex`/`Background` arm
        // docs above for the "not a safety net" framing) — unreachable in
        // practice, `expand_shorthand_into` expands it to the 6
        // `FontStyle`/`FontVariantCaps`/`FontWeight`/`FontSize`-or-
        // `FontSizeRelative`/`LineHeight`/`FontFamily` longhands before
        // `apply_value` ever sees it. Delegates to the 6 longhand arms
        // above (rather than duplicating their logic) so the
        // inheritance-seed reads (`FontWeight`'s `bolder`/`lighter`,
        // `FontSizeRelative`'s `larger`/`smaller`) behave exactly as if
        // the shorthand had been expanded — `target` still holds the
        // parent seeds here since no longhand arm ran yet for this node.
        // `border-style` / `border-width` / `border-color` shorthands
        // decompose into their side longhands (same shape as `Font`
        // below; reached when rule.rs expansion is bypassed).
        PropertyValue::BorderStyle(sides) => {
            apply_value(PropertyValue::BorderTopStyle(sides.top), target);
            apply_value(PropertyValue::BorderRightStyle(sides.right), target);
            apply_value(PropertyValue::BorderBottomStyle(sides.bottom), target);
            apply_value(PropertyValue::BorderLeftStyle(sides.left), target);
        }
        PropertyValue::BorderWidth(sides) => {
            apply_value(PropertyValue::BorderTopWidth(sides.top), target);
            apply_value(PropertyValue::BorderRightWidth(sides.right), target);
            apply_value(PropertyValue::BorderBottomWidth(sides.bottom), target);
            apply_value(PropertyValue::BorderLeftWidth(sides.left), target);
        }
        PropertyValue::BorderColor(sides) => {
            apply_value(PropertyValue::BorderTopColor(sides.top), target);
            apply_value(PropertyValue::BorderRightColor(sides.right), target);
            apply_value(PropertyValue::BorderBottomColor(sides.bottom), target);
            apply_value(PropertyValue::BorderLeftColor(sides.left), target);
        }
        PropertyValue::Font(shorthand) => {
            apply_value(PropertyValue::FontStyle(shorthand.style), target);
            apply_value(PropertyValue::FontVariantCaps(shorthand.variant), target);
            apply_value(PropertyValue::FontWeight(shorthand.weight), target);
            match shorthand.size {
                crate::property::FontShorthandSize::Absolute(length) => {
                    apply_value(PropertyValue::FontSize(length), target);
                }
                crate::property::FontShorthandSize::Relative(relative) => {
                    apply_value(PropertyValue::FontSizeRelative(relative), target);
                }
            }
            apply_value(PropertyValue::LineHeight(shorthand.line_height), target);
            apply_value(PropertyValue::FontFamily(shorthand.family), target);
        }
        // CSS Images Module Level 3 §5.1. non-inherited, computed value =
        // specified keyword — simple assignment, no length payload
        // (`BackgroundRepeat` arm と同じ shape)。
        PropertyValue::ObjectFit(v) => target.object_fit = v,
        // CSS Images Module Level 3 §5.2. non-inherited、`<length-percentage>`
        // を含むため specified 表現のまま格納 — 絶対化は phase 3
        // (`SpecifiedValues::absolutize_with`) に委ねる (`BackgroundPosition`
        // arm と同じ shape)。
        PropertyValue::ObjectPosition(v) => target.object_position = v,
        // CSS Color 4 §3.3. non-inherited — simple assignment, **not**
        // clamped here (`PropertyValue::Opacity` doc's "specified preserves,
        // computed clamps" note). The `[0,1]` clamp happens in
        // `SpecifiedValues::absolutize_with` (phase 3), not at winner
        // application time.
        PropertyValue::Opacity(v) => target.opacity = v,
        // CSS Compositing and Blending Level 1 §3.4.2. non-inherited,
        // keyword-only — simple assignment.
        PropertyValue::Isolation(v) => target.isolation = v,
        // CSS Compositing and Blending Level 1 §3.4.1. non-inherited,
        // keyword-only — simple assignment.
        PropertyValue::MixBlendMode(v) => target.mix_blend_mode = v,
        // CSS Masking Level 1 §7.1/§5.1. non-inherited — simple assignment,
        // no phase-3 transform (`MaskImage`/`ClipPath` doc's scope notes).
        PropertyValue::MaskImage(v) => target.mask_image = v,
        PropertyValue::ClipPath(v) => target.clip_path = v,
        // CSS Transforms Level 1 §4 / CSS Filter Effects Level 1 §5.
        // non-inherited — simple assignment, no phase-3 transform
        // (`TransformFunction`/`FilterFunction` doc's scope notes).
        PropertyValue::Transform(v) => target.transform = v,
        PropertyValue::Filter(v) => target.filter = v,
        // CSS Tables 3 §4 table-layout。non-inherited、computed value =
        // specified keyword — simple assignment、length payload 無し
        // (`BackgroundRepeat` arm と同じ shape)。
        PropertyValue::TableLayout(v) => target.table_layout = v,
        // CSS Tables 3 §6 border-collapse。inherited だが keyword のため
        // inherit 解決は `SpecifiedValues::inherit_from` の素朴なコピーが担い、
        // ここは winner の単純代入 (`Visibility` arm と同じ shape)。
        PropertyValue::BorderCollapse(v) => target.border_collapse = v,
        // CSS Tables 3 §6.1 border-spacing。inherited だが `<length>` のため
        // inherit 解決は `SpecifiedValues::inherit_from` の lift
        // (`lift_border_spacing`、`tab_size` の `Length` arm と同じ) が担い、
        // ここは winner の単純代入 (`BorderCollapse` arm と同じ shape —
        // 絶対化は phase 3 `finalize` の仕事)。
        PropertyValue::BorderSpacing(v) => target.border_spacing = v,
        // CSS Tables 3 §7 caption-side。inherited だが keyword のため
        // inherit 解決は `SpecifiedValues::inherit_from` の素朴なコピーが担い、
        // ここは winner の単純代入 (`Visibility` arm と同じ shape)。
        PropertyValue::CaptionSide(v) => target.caption_side = v,
        // CSS Tables 3 §8 empty-cells。inherited だが keyword のため
        // inherit 解決は `SpecifiedValues::inherit_from` の素朴なコピーが担い、
        // ここは winner の単純代入 (`Visibility` arm と同じ shape)。
        PropertyValue::EmptyCells(v) => target.empty_cells = v,
        // New Text 3 / Writing Modes 3 / Text Decoration 4 properties with no
        // element staging field yet (parsing only). The keyword-only ones
        // (`LineBreak`...`UnicodeBidi`, `TextDecorationSkipInk`...
        // `TextUnderlinePosition`) carry no length. `TextDecorationThickness`
        // remains parsing-only; `TextDecorationInset` is handled by the
        // ordinary specified/computed element path above.
        PropertyValue::TextAlignAll(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextUnderlinePosition(_) => {}
        // CSS Paged Media 3 §8.1: retain the non-inherited named-page value
        // in the specified staging bag; the page driver consumes it when it
        // selects the next page context.
        PropertyValue::Page(value) => target.page = value,
        // CSS Multi-column Layout 1: both longhands are non-inherited and
        // retain their specified representations until finalization.
        PropertyValue::ColumnCount(value) => target.column_count = value,
        PropertyValue::ColumnWidth(value) => target.column_width = value,
        // `columns` is expanded by `rule::expand_shorthand_into`; keep this
        // arm defensive for callers that construct declarations directly.
        // cov:ignore: direct unexpanded shorthand callers are defensive-only.
        PropertyValue::Columns(value) => {
            target.column_count = value.count;
            target.column_width = value.width;
        }
        // These values are resolved before ordinary winners reach this
        // function. Keeping an explicit no-op makes direct internal callers
        // panic-free without allowing raw deferred data into a computed field.
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
mod tests {
    use super::*;
    use crate::cascade::test_support::*;
    use crate::cascade::{cascade, cascade_with_media_context_for_page};
    use crate::computed::{ComputedValues, INITIAL_FONT_SIZE_PX};
    use crate::media::MediaContext;
    use crate::property::CssColor;
    use crate::property::DisplayValue;
    use crate::property::{
        Border, BorderColor, BorderStyle, CalcLengthPercentage, ContentComponent,
        GridAreaShorthand, GridAutoFlowValue, GridLineValue, GridShorthand, GridTemplateAreasValue,
        GridTemplateTracks, Length, LengthOrAuto, ListStylePosition, ListStyleType, Outline,
        OutlineColor, OutlineStyle, OverflowValue, OverflowXY, PageValue, PositionValue,
        PropertyKey, PropertyValue, Sides, TextDecorationColor, TextDecorationLine,
        TextDecorationShorthand, TextDecorationStyle, TextDecorationThickness, TextShadowColor,
        empty_counter_entries, initial_grid_auto_track_list,
    };
    use crate::resolve::{
        ComputedBorder, ComputedBorderRadius, ComputedBoxShadowItem, ComputedLength,
        ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLineHeight,
        ComputedTabSize, ComputedTextDecorationInset, ComputedTextShadow,
    };
    use crate::ruletree::{RuleTree, build_rule_tree};
    use crate::test_dom::TestDoc;
    use smol_str::SmolStr;

    #[test]
    fn min_block_size_maps_to_the_authored_block_axis() {
        let horizontal = cascade_doc(
            "",
            "div",
            Some("min-block-size: 40px; writing-mode: horizontal-tb"),
        );
        assert_eq!(horizontal.min_width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(
            horizontal.min_height,
            ComputedLengthPercentageOrAuto::Px(40.0)
        );
        assert_eq!(
            horizontal.min_block_size,
            Some(ComputedLengthPercentageOrAuto::Px(40.0))
        );

        let vertical = cascade_doc(
            "",
            "div",
            Some("min-block-size: 40px; writing-mode: vertical-rl"),
        );
        assert_eq!(vertical.min_width, ComputedLengthPercentageOrAuto::Px(40.0));
        assert_eq!(vertical.min_height, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(
            vertical.min_block_size,
            Some(ComputedLengthPercentageOrAuto::Px(40.0))
        );

        let physical = cascade_doc("", "div", Some("min-height: 20px"));
        assert_eq!(physical.min_width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(
            physical.min_height,
            ComputedLengthPercentageOrAuto::Px(20.0)
        );
        assert_eq!(physical.min_block_size, None);
    }

    #[test]
    fn inheritance_walk_child_from_parent_element() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red }");
        let p = doc.push_element(0, "p", None);
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        // <p> が red、<span> は inherit で red
        assert_eq!(r.computed[p].color, RED);
        assert_eq!(r.computed[span].color, RED);
    }

    #[test]
    fn inheritance_falls_through_to_initial_when_no_rule_matches() {
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.color, ComputedValues::initial().color);
    }

    #[test]
    fn text_node_inherits_from_element_parent() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("color: red"));
        let t = doc.push_text(p, "Hi");
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        assert_eq!(r.computed[p].color, RED);
        assert_eq!(r.computed[t].color, RED);
    }

    fn cascade_parent_child(
        parent_tag: &str,
        parent_inline: Option<&str>,
        child_tag: &str,
        child_inline: Option<&str>,
    ) -> (ComputedValues, ComputedValues) {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, parent_tag, parent_inline);
        let child = doc.push_element(parent, child_tag, child_inline);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        (r.computed[parent].clone(), r.computed[child].clone())
    }

    #[test]
    fn declared_em_and_inherited_em_produce_different_computed_font_sizes() {
        let (div_i, span_i) = cascade_parent_child(
            "div",
            Some("font-size: 1.5em"),
            "span",
            Some("font-size: 1.5em"),
        );
        let (div_ii, span_ii) = cascade_parent_child("div", Some("font-size: 1.5em"), "span", None);

        // 親はどちらも initial 16px に対する 1.5em = 24px。
        assert_eq!(div_i.font_size, ComputedLength(24.0));
        assert_eq!(div_ii.font_size, ComputedLength(24.0));

        // (i) 宣言由来 — 自 node で再度 1.5 倍される。
        assert_eq!(span_i.font_size, ComputedLength(36.0));
        // (ii) inherit 由来 — 親の computed value がそのまま。
        assert_eq!(span_ii.font_size, ComputedLength(24.0));
        assert_ne!(
            span_i.font_size, span_ii.font_size,
            "declared em と inherited em が同値になるのが 082k Rationale 1 の live bug"
        );
    }

    #[test]
    fn em_font_size_compounds_across_cascade_levels() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-size: 1.5em"));
        let b = doc.push_element(a, "div", Some("font-size: 1.5em"));
        let c = doc.push_element(b, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].font_size, ComputedLength(24.0));
        assert_eq!(r.computed[b].font_size, ComputedLength(36.0));
        // 孫は宣言が無いので親の computed value を継承 (再乗算しない)。
        assert_eq!(r.computed[c].font_size, ComputedLength(36.0));
    }

    #[test]
    fn box_property_em_resolves_against_own_computed_font_size() {
        let cv = cascade_doc(
            "",
            "div",
            Some(
                "font-size: 20px; padding: 2em; margin-left: 1.5em; border-top-width: 0.5em; border-top-style: solid; width: 3em; line-height: 1.2em",
            ),
        );
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(30.0));
        assert_eq!(cv.border.top.width, ComputedLength(10.0));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(60.0));
        assert_eq!(
            cv.line_height,
            ComputedLineHeight::Length(ComputedLength(24.0))
        );
    }

    #[test]
    fn rem_on_root_element_resolves_against_initial_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: 2rem"));
        assert_eq!(cv.font_size, ComputedLength(32.0));
    }

    #[test]
    fn rem_below_root_element_resolves_against_root_computed_font_size() {
        let (root, child) = cascade_parent_child(
            "html",
            Some("font-size: 20px"),
            "p",
            Some("font-size: 2rem"),
        );
        assert_eq!(root.font_size, ComputedLength(20.0));
        assert_eq!(child.font_size, ComputedLength(40.0));
    }

    #[test]
    fn rem_on_root_element_box_property_uses_own_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: 20px; padding: 2rem"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
    }

    #[test]
    fn lh_resolves_against_own_computed_line_height() {
        // font-size は initial 16px、line-height: 2 (Number) → used = 32px。
        let cv = cascade_doc("", "div", Some("line-height: 2; padding: 1.5lh"));
        assert_eq!(
            cv.line_height,
            ComputedLineHeight::Number(2.0),
            "line-height 自身は Number のまま computed 層に残る (spec 上 load-bearing)"
        );
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(48.0))); // 1.5 * 32
    }

    #[test]
    fn lh_falls_back_to_zero_when_own_line_height_is_normal() {
        let cv = cascade_doc("", "div", Some("padding: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    #[test]
    fn margin_lh_falls_back_to_zero_not_auto_when_line_height_normal() {
        let cv = cascade_doc("", "div", Some("margin-top: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0),
            "margin-top: 1lh under line-height: normal must be Px(0.0), not Auto \
             (Auto would trigger real auto-margin layout with no spec basis)"
        );
    }

    #[test]
    fn rlh_on_root_element_matches_child_root_line_height_basis() {
        let mut doc = TestDoc::new();
        let html = doc.push_element(
            0,
            "html",
            Some("font-size: 20px; line-height: 2; padding: 1rlh"),
        );
        // Deliberately different own font-size/line-height, to prove `rlh`
        // does not read the child's own metrics.
        let p = doc.push_element(
            html,
            "p",
            Some("font-size: 100px; line-height: 5; padding: 1rlh"),
        );
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // root's own used line-height: 2 * 20px = 40px.
        assert_eq!(
            r.computed[html].line_height,
            ComputedLineHeight::Number(2.0)
        );
        assert_eq!(
            r.computed[html].padding,
            Sides::all(ComputedLengthPercentage::Px(40.0)),
        );
        // child's `1rlh` uses the *same* 40px basis, not its own (100px,
        // Number(5) → 500px) line-height.
        assert_eq!(
            r.computed[p].padding,
            Sides::all(ComputedLengthPercentage::Px(40.0)),
            "rlh must be the same tree-global constant on the root and on a descendant"
        );
    }

    #[test]
    fn rlh_falls_back_to_zero_when_root_line_height_is_normal() {
        let (root, child) = cascade_parent_child("html", None, "p", Some("padding: 1rlh"));
        assert_eq!(root.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.padding, Sides::all(ComputedLengthPercentage::Px(0.0)),);
    }

    #[test]
    fn line_height_lh_self_reference_uses_parent_not_own_metrics() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("line-height: 2"), // own font-size 16px (initial) → used 32px
            "span",
            Some("font-size: 50px; line-height: 1lh"),
        );
        assert_eq!(parent.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(
            child.line_height,
            ComputedLineHeight::Length(ComputedLength(32.0)),
            "1lh on line-height itself must resolve against the parent's used \
             line-height (32px), not the child's own font-size (50px)"
        );
    }

    #[test]
    fn line_height_lh_self_reference_falls_back_to_normal_when_parent_is_normal() {
        let (parent, child) = cascade_parent_child("div", None, "span", Some("line-height: 1lh"));
        assert_eq!(parent.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.line_height, ComputedLineHeight::Normal);
    }

    #[test]
    fn line_height_rlh_in_line_height_uses_root_not_immediate_parent() {
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2")); // root used = 40px
        let middle = doc.push_element(root, "div", Some("font-size: 12px; line-height: 4")); // middle used = 48px
        let leaf = doc.push_element(middle, "span", Some("line-height: 1rlh"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(
            r.computed[root].line_height,
            ComputedLineHeight::Number(2.0)
        );
        assert_eq!(
            r.computed[middle].line_height,
            ComputedLineHeight::Number(4.0)
        );
        assert_eq!(
            r.computed[leaf].line_height,
            ComputedLineHeight::Length(ComputedLength(40.0)),
            "1rlh on line-height itself must use the root's used line-height \
             (40px), not the immediate parent's (48px) — rlh is not self-referential"
        );
    }

    #[test]
    fn line_height_lh_self_reference_on_root_element_is_always_normal() {
        let cv = cascade_doc("", "html", Some("line-height: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    }

    #[test]
    fn font_size_lh_declaration_is_no_longer_parse_dropped_and_can_win_cascade() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, "* { font-size: 12px } p { font-size: 1lh }");
        // `* { font-size: 12px }` also matches `html`, but the inline
        // declaration below beats it (inline specificity exceeds any
        // selector's). html: font-size 20px, line-height: 2 → used
        // line-height 40px.
        let html = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2"));
        let p = doc.push_element(html, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_size,
            ComputedLength(40.0), // 1 * parent's (html's) used line-height (2 * 20px)
            "p's own `font-size: 1lh` (specificity 0,0,1) must win over \
             `* {{ font-size: 12px }}` (specificity 0,0,0); before the fix, \
             1lh was parse-dropped, silently leaving only the universal-selector \
             declaration as a cascade candidate"
        );
    }

    #[test]
    fn font_size_lh_resolves_against_parent_used_line_height() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("line-height: 2"), // own font-size 16px (initial) → used 32px
            "span",
            Some("font-size: 1.5lh"),
        );
        assert_eq!(parent.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(child.font_size, ComputedLength(48.0)); // 1.5 * 32
    }

    #[test]
    fn font_size_lh_falls_back_to_initial_when_parent_line_height_is_normal() {
        let (parent, child) = cascade_parent_child("div", None, "span", Some("font-size: 1lh"));
        assert_eq!(parent.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.font_size, ComputedLength(INITIAL_FONT_SIZE_PX));
    }

    #[test]
    fn font_size_lh_on_root_element_falls_back_to_initial() {
        let cv = cascade_doc("", "html", Some("font-size: 1lh"));
        assert_eq!(cv.font_size, ComputedLength(INITIAL_FONT_SIZE_PX));
    }

    #[test]
    fn font_size_rlh_uses_root_not_immediate_parent() {
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2")); // root used = 40px
        let middle = doc.push_element(root, "div", Some("font-size: 12px; line-height: 4")); // middle used = 48px
        let leaf = doc.push_element(middle, "span", Some("font-size: 1rlh"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(
            r.computed[root].line_height,
            ComputedLineHeight::Number(2.0)
        );
        assert_eq!(
            r.computed[middle].line_height,
            ComputedLineHeight::Number(4.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[leaf].font_size,
            ComputedLength(40.0),
            "font-size: 1rlh must use the root's used line-height (40px), not \
             the immediate parent's (48px) — rlh is not self-referential"
        );
    }

    #[test]
    fn font_size_lh_uses_immediate_parent_not_root() {
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2")); // root used = 40px
        let middle = doc.push_element(root, "div", Some("font-size: 12px; line-height: 4")); // middle used = 48px
        let leaf = doc.push_element(middle, "span", Some("font-size: 1lh"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[leaf].font_size,
            ComputedLength(48.0),
            "font-size: 1lh must use the *immediate parent's* used line-height \
             (48px), not the root's (40px) — lh is self-referential, unlike rlh"
        );
    }

    #[test]
    fn non_element_node_under_document_does_not_establish_rem_context() {
        let mut doc = TestDoc::new();
        let text = doc.push_text(0, "bare text");
        let html = doc.push_element(0, "html", Some("font-size: 20px"));
        let child = doc.push_element(html, "p", Some("padding: 1rem"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // 非 element node は cascade winner を持たないので全 field が initial。
        assert_eq!(r.computed[text], ComputedValues::initial());
        // 兄弟 element 側の subtree は自身の root element (html) 基準で解決される
        // — text node の存在に影響されない。
        assert_eq!(r.computed[html].font_size, ComputedLength(20.0));
        assert_eq!(
            r.computed[child].padding,
            Sides::all(ComputedLengthPercentage::Px(20.0))
        );
    }

    #[test]
    fn rem_ignores_intermediate_font_sizes() {
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", Some("font-size: 20px"));
        let mid = doc.push_element(html, "div", Some("font-size: 40px"));
        let leaf = doc.push_element(mid, "span", Some("padding: 1rem"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[leaf].padding,
            Sides::all(ComputedLengthPercentage::Px(20.0)),
            "rem は root element (20px) 基準 — 直近の親 (40px) ではない"
        );
    }

    #[test]
    fn bolder_resolves_against_parent_computed_weight_through_staging() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-weight: 700"),
            "span",
            Some("font-weight: bolder"),
        );
        assert_eq!(parent.font_weight, 700.0);
        assert_eq!(
            child.font_weight, 900.0,
            "bolder は親の computed 700 に対して解決される (initial 400 起点なら 700 になる)"
        );

        // lighter 側も同じ経路を通る (700 → 400)。
        let (_, lighter) = cascade_parent_child(
            "div",
            Some("font-weight: 700"),
            "span",
            Some("font-weight: lighter"),
        );
        assert_eq!(lighter.font_weight, 400.0);
    }

    #[test]
    fn bolder_chain_compounds_through_staging() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-weight: bolder"));
        let b = doc.push_element(a, "div", Some("font-weight: bolder"));
        let c = doc.push_element(b, "div", Some("font-weight: bolder"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].font_weight, 700.0);
        assert_eq!(r.computed[b].font_weight, 900.0);
        assert_eq!(r.computed[c].font_weight, 900.0);
    }

    #[test]
    fn larger_resolves_against_parent_computed_font_size_through_staging() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-size: 20px"),
            "span",
            Some("font-size: larger"),
        );
        assert_eq!(parent.font_size, ComputedLength(20.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            child.font_size,
            ComputedLength(24.0),
            "larger は親の computed 20px に対して解決される (initial 16px 起点なら 19.2px になる)"
        );

        // smaller 側も同じ経路を通る (20px → 20/1.2px)。
        let (_, smaller) = cascade_parent_child(
            "div",
            Some("font-size: 20px"),
            "span",
            Some("font-size: smaller"),
        );
        assert_eq!(smaller.font_size, ComputedLength(20.0 / 1.2));
    }

    #[test]
    fn larger_chain_compounds_through_staging() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-size: larger"));
        let b = doc.push_element(a, "div", Some("font-size: larger"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // literal (`19.2`) ではなく式 (`16.0 * 1.2`) で期待値を書く — 実装の
        // 計算式をそのまま mirror し、f32 の最終 bit まで一致させる
        // (`resolve_relative_font_size` の `RATIO` 定数と同じ乗算)。
        assert_eq!(r.computed[a].font_size, ComputedLength(16.0 * 1.2));
        assert_eq!(r.computed[b].font_size, ComputedLength(16.0 * 1.2 * 1.2));
    }

    #[test]
    fn larger_on_root_element_resolves_against_initial_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: larger"));
        assert_eq!(cv.font_size, ComputedLength(19.2));
    }

    #[test]
    fn pt_is_absolutized_at_cascade() {
        let cv = cascade_doc("", "div", Some("font-size: 15pt; padding-top: 9pt"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_ne!(
            cv.font_size,
            ComputedValues::initial().font_size,
            "initial と一致する期待値は parse 側の drop を検出できない"
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(12.0));
    }

    #[test]
    fn line_height_percentage_is_resolved_at_declaring_element() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-size: 20px; line-height: 150%"),
            "span",
            Some("font-size: 10px"),
        );
        assert_eq!(
            parent.line_height,
            ComputedLineHeight::Length(ComputedLength(30.0))
        );
        assert_eq!(
            child.line_height,
            ComputedLineHeight::Length(ComputedLength(30.0)),
            "子は 30px をそのまま継承する (15px に再解決しない)"
        );
    }

    #[test]
    fn line_height_number_stays_unitless_through_computed_layer() {
        let (_, child) = cascade_parent_child(
            "div",
            Some("font-size: 20px; line-height: 1.5"),
            "span",
            Some("font-size: 10px"),
        );
        assert_eq!(child.line_height, ComputedLineHeight::Number(1.5));
    }

    #[test]
    fn absolutization_is_independent_of_declaration_order() {
        let a = cascade_doc("", "div", Some("font-size: 20px; padding: 2em"));
        let b = cascade_doc("", "div", Some("padding: 2em; font-size: 20px"));
        assert_eq!(a.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
        assert_eq!(a.padding, b.padding);
        assert_eq!(a.font_size, b.font_size);
    }

    #[test]
    fn author_display_flex_and_grid_compute_through_cascade() {
        // Cascade output (`ComputedValues.display`) is read directly by
        // raikiri-paint's `walk.rs` (independent of raikiri-dom's taffy
        // bridge), so `display: flex` / `display: grid` reaching a
        // `DisplayValue::Flex` / `DisplayValue::Grid` computed value here is
        // observable to that consumer on its own, not only once the taffy
        // bridge exists.
        let flex_cv = cascade_with_ua("", "div { display: flex }", "div", None);
        assert_eq!(flex_cv.display, DisplayValue::Flex);
        let grid_cv = cascade_with_ua("", "div { display: grid }", "div", None);
        assert_eq!(grid_cv.display, DisplayValue::Grid);
    }

    #[test]
    fn author_display_flow_root_computes_through_cascade() {
        // End-to-end pipeline check (parse -> cascade -> ComputedValues) for
        // the standalone CSS Display 3 `flow-root` keyword.
        let cv = cascade_with_ua("", "div { display: flow-root }", "div", None);
        assert_eq!(cv.display, DisplayValue::FlowRoot);
    }

    #[test]
    fn author_display_list_item_computes_through_cascade() {
        // End-to-end pipeline check (parse -> cascade -> ComputedValues) for
        // `display: list-item` — keyword-acceptance only, sibling of
        // `author_display_flex_and_grid_compute_through_cascade` above.
        let cv = cascade_with_ua("", "li { display: list-item }", "li", None);
        assert_eq!(cv.display, DisplayValue::ListItem);
    }

    #[test]
    fn author_display_contents_computes_through_cascade() {
        // CSS Display 3 §2.5 `contents` — same end-to-end pipeline check as
        // `author_display_flex_and_grid_compute_through_cascade` above.
        // `ComputedValues.display` reaching `DisplayValue::Contents` here
        // is this crate's whole scope for this keyword — see
        // `DisplayValue::Contents`'s doc for the known consumer-side box
        // generation gap this does not (and should not) work around.
        let cv = cascade_with_ua("", "div { display: contents }", "div", None);
        assert_eq!(cv.display, DisplayValue::Contents);
    }

    #[test]
    fn author_flex_container_longhands_compute_through_cascade() {
        // End-to-end pipeline check (parse -> cascade -> ComputedValues) for
        // the individual flex-container properties — sibling of
        // `author_display_flex_and_grid_compute_through_cascade` above.
        let cv = cascade_with_ua(
            "",
            "div { display: flex; flex-direction: column; flex-wrap: wrap; \
             justify-content: space-between; align-items: center; \
             align-content: flex-end; row-gap: 10px; column-gap: 5%; }",
            "div",
            None,
        );
        assert_eq!(
            cv.flex_direction,
            crate::property::FlexDirectionValue::Column
        );
        assert_eq!(cv.flex_wrap, crate::property::FlexWrapValue::Wrap);
        assert_eq!(
            cv.justify_content,
            crate::property::ContentAlignmentValue::SpaceBetween
        );
        assert_eq!(cv.align_items, crate::property::SelfAlignmentValue::Center);
        assert_eq!(
            cv.align_content,
            crate::property::ContentAlignmentValue::FlexEnd
        );
        assert_eq!(
            cv.row_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Px(10.0)
        );
        assert_eq!(
            cv.column_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Percent(5.0)
        );
    }

    #[test]
    fn author_flex_item_longhands_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { flex-grow: 2; flex-shrink: 0; flex-basis: 50%; align-self: flex-end; }",
            "div",
            None,
        );
        assert_eq!(cv.flex_grow, 2.0);
        assert_eq!(cv.flex_shrink, 0.0);
        assert_eq!(
            cv.flex_basis,
            crate::resolve::ComputedFlexBasis::Percent(50.0)
        );
        assert_eq!(
            cv.align_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::FlexEnd)
        );
    }

    #[test]
    fn author_multicol_longhands_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { column-count: 3; column-width: 2em; }",
            "div",
            None,
        );
        assert_eq!(cv.column_count, crate::property::ColumnCountValue::Count(3));
        assert_eq!(
            cv.column_width,
            crate::resolve::ComputedColumnWidth::Px(32.0)
        );
    }

    #[test]
    fn author_multicol_shorthand_fans_out_through_cascade() {
        let cv = cascade_with_ua("", "div { columns: 3 2em; }", "div", None);
        assert_eq!(cv.column_count, crate::property::ColumnCountValue::Count(3));
        assert_eq!(
            cv.column_width,
            crate::resolve::ComputedColumnWidth::Px(32.0)
        );
    }

    #[test]
    fn author_flex_basis_intrinsic_keywords_compute_through_cascade() {
        // `min-content` / `max-content` / bare `fit-content` survive the
        // full stylesheet -> cascade path as distinct computed keywords
        // (WPT `flex-basis-valid.html`; rendering approximation lives at
        // the taffy bridge, `bridge_flex` doc).
        let cv = cascade_doc("", "div", Some("flex-basis: min-content"));
        assert_eq!(cv.flex_basis, crate::resolve::ComputedFlexBasis::MinContent);
        let cv = cascade_doc("", "div", Some("flex-basis: max-content"));
        assert_eq!(cv.flex_basis, crate::resolve::ComputedFlexBasis::MaxContent);
        let cv = cascade_doc("", "div", Some("flex-basis: fit-content"));
        assert_eq!(cv.flex_basis, crate::resolve::ComputedFlexBasis::FitContent);
    }

    #[test]
    fn author_flex_shorthand_expands_into_3_longhands_through_cascade() {
        // Proves `crate::rule::expand_shorthand_into`'s `Flex` arm is
        // actually wired into the real parse -> cascade pipeline (not just
        // unit-tested at the parser/expansion-function level) — same
        // end-to-end intent as `flex_shorthand_*` tests in `property.rs`,
        // but through the full stylesheet -> cascade path.
        let cv = cascade_with_ua("", "div { flex: 2 3 10%; }", "div", None);
        assert_eq!(cv.flex_grow, 2.0);
        assert_eq!(cv.flex_shrink, 3.0);
        assert_eq!(
            cv.flex_basis,
            crate::resolve::ComputedFlexBasis::Percent(10.0)
        );
    }

    #[test]
    fn author_gap_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua("", "div { gap: 10px 20px; }", "div", None);
        assert_eq!(
            cv.row_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Px(10.0)
        );
        assert_eq!(
            cv.column_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Px(20.0)
        );
    }

    #[test]
    fn author_place_content_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { place-content: center space-between; }",
            "div",
            None,
        );
        assert_eq!(
            cv.align_content,
            crate::property::ContentAlignmentValue::Center
        );
        assert_eq!(
            cv.justify_content,
            crate::property::ContentAlignmentValue::SpaceBetween
        );
    }

    #[test]
    fn author_grid_template_columns_track_list_absolutizes_through_cascade() {
        // `2em` at `font-size: 20px` → `40px` — proves phase 3 absolutizes
        // `<length-percentage>` inside the track list (not just passes the
        // specified value through), sibling of `author_flex_item_longhands_compute_through_cascade`
        // above.
        let cv = cascade_with_ua(
            "",
            "div { font-size: 20px; grid-template-columns: 2em 1fr auto; }",
            "div",
            None,
        );
        let crate::resolve::ComputedGridTemplateTracks::List(list) = cv.grid_template_columns
        else {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            panic!("expected a track list");
        };
        assert_eq!(
            list.components,
            vec![
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Px(40.0)
                    )
                ),
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Flex(1.0)
                    )
                ),
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Auto
                    )
                ),
            ]
        );
    }

    #[test]
    fn author_grid_template_rows_and_grid_auto_rows_compute_through_cascade() {
        // Sibling of `author_grid_template_columns_track_list_absolutizes_through_cascade`
        // above — `grid-template-rows`/`grid-auto-rows` share the parser and
        // `apply_value` arm shape with their `-columns` counterparts but
        // were never independently exercised through the cascade pipeline.
        let cv = cascade_with_ua(
            "",
            "div { grid-template-rows: 1fr 2fr; grid-auto-rows: min-content; }",
            "div",
            None,
        );
        let crate::resolve::ComputedGridTemplateTracks::List(rows) = cv.grid_template_rows else {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            panic!("expected a track list");
        };
        assert_eq!(
            rows.components,
            vec![
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Flex(1.0)
                    )
                ),
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Flex(2.0)
                    )
                ),
            ]
        );
        assert_eq!(
            cv.grid_auto_rows,
            std::sync::Arc::new(vec![crate::resolve::ComputedGridTrackSize::Breadth(
                crate::resolve::ComputedGridTrackBreadth::MinContent
            )])
        );
    }

    #[test]
    fn author_grid_template_areas_computes_through_cascade() {
        let cv = cascade_with_ua(
            "",
            r#"div { grid-template-areas: "header header" "nav main"; }"#,
            "div",
            None,
        );
        let crate::property::GridTemplateAreasValue::Areas(areas) = cv.grid_template_areas else {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            panic!("expected parsed areas");
        };
        assert_eq!(areas.row_count, 2);
        assert_eq!(areas.column_count, 2);
        assert!(areas.areas.iter().any(|a| a.name == "header"));
        assert!(areas.areas.iter().any(|a| a.name == "nav"));
        assert!(areas.areas.iter().any(|a| a.name == "main"));
    }

    #[test]
    fn author_grid_auto_flow_and_placement_longhands_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { grid-auto-flow: column dense; grid-row-start: 2; \
             grid-column-start: span 3; }",
            "div",
            None,
        );
        assert_eq!(
            cv.grid_auto_flow,
            crate::property::GridAutoFlowValue::ColumnDense
        );
        assert_eq!(cv.grid_row_start, crate::property::GridLineValue::Line(2));
        assert_eq!(
            cv.grid_column_start,
            crate::property::GridLineValue::Span(3)
        );
    }

    #[test]
    fn author_grid_row_shorthand_expands_into_2_longhands_through_cascade() {
        // Proves `crate::rule::expand_shorthand_into`'s `GridRow` arm is
        // wired into the real parse -> cascade pipeline, sibling of
        // `author_flex_shorthand_expands_into_3_longhands_through_cascade`
        // above.
        let cv = cascade_with_ua("", "div { grid-row: 2 / 5; }", "div", None);
        assert_eq!(cv.grid_row_start, crate::property::GridLineValue::Line(2));
        assert_eq!(cv.grid_row_end, crate::property::GridLineValue::Line(5));
    }

    #[test]
    fn author_grid_column_shorthand_omitted_second_copies_ident_through_cascade() {
        let cv = cascade_with_ua("", "div { grid-column: content; }", "div", None);
        assert_eq!(
            cv.grid_column_start,
            crate::property::GridLineValue::Named("content".into())
        );
        assert_eq!(
            cv.grid_column_end,
            crate::property::GridLineValue::Named("content".into())
        );
    }

    #[test]
    fn author_justify_items_and_justify_self_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { justify-items: center; justify-self: end; }",
            "div",
            None,
        );
        assert_eq!(
            cv.justify_items,
            crate::property::SelfAlignmentValue::Center
        );
        assert_eq!(
            cv.justify_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::End)
        );
    }

    #[test]
    fn author_place_items_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua("", "div { place-items: start end; }", "div", None);
        assert_eq!(cv.align_items, crate::property::SelfAlignmentValue::Start);
        assert_eq!(cv.justify_items, crate::property::SelfAlignmentValue::End);
    }

    #[test]
    fn author_place_self_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua("", "div { place-self: center; }", "div", None);
        assert_eq!(
            cv.align_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::Center)
        );
        assert_eq!(
            cv.justify_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::Center)
        );
    }

    #[test]
    fn non_inherited_display_child_starts_from_initial_not_parent() {
        // <div> が UA CSS で display: block、その子 <span> は自身 rule がなく、
        // display は non-inherited なので initial (Inline) となる
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ""); // author 空
        let div = doc.push_element(0, "div", None);
        let span = doc.push_element(div, "span", None);

        let mut tree = build_rule_tree(&doc);
        tree.add_stylesheet("div { display: block } span { }", Origin::UserAgent);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].display, DisplayValue::Block);
        assert_eq!(r.computed[span].display, DisplayValue::Inline);
    }

    #[test]
    fn background_color_wired_through_cascade_from_inline_style() {
        // <div style="background-color: red"> → ComputedValues.background_color
        // に RED が届く。parser → PropertyValue::BackgroundColor → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。sibling `color` の wire-through
        // pattern (`type_selector_applies_color`) を踏襲。
        let cv = cascade_doc("", "div", Some("background-color: red"));
        assert_eq!(cv.background_color, RED);
    }

    #[test]
    fn background_color_is_non_inherited_child_starts_from_initial_transparent() {
        // CSS Backgrounds 3 §2.2 "Inheritance: no"。<p style='background-color:red'>
        // の子 <span> は自身 rule がなく、background_color は initial (transparent)。
        // sibling pattern (display / counter-* / content / string-set /
        // position の non-inheritance test 群を踏襲)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-color: red"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_color, RED,
            "parent should carry its own background-color"
        );
        assert_eq!(
            r.computed[span].background_color,
            CssColor::TRANSPARENT,
            "child should not inherit background-color (initial: transparent)"
        );
    }

    #[test]
    fn background_color_transparent_keyword_resolves_to_zero_alpha() {
        // CSS Color 4 §6.3 "The transparent keyword": `transparent`
        // = rgba(0, 0, 0, 0)。CssColor::TRANSPARENT が cascade winner として
        // per-node に到達することを check (parse_color の transparent Ident branch
        // と CssColor::TRANSPARENT const の regression canary)。
        let cv = cascade_doc("", "div", Some("background-color: transparent"));
        assert_eq!(cv.background_color, CssColor::TRANSPARENT);
        assert_eq!(cv.background_color.a, 0);
    }

    #[test]
    fn line_height_wired_through_cascade_from_inline_style() {
        // <p style="line-height: 1.5"> → ComputedValues.line_height に
        // LineHeight::Number(1.5) が届く。parser → PropertyValue::LineHeight
        // → apply_value → ComputedValues の end-to-end 疎通 smoke
        // (font-size / color と同じ inherited property pattern)。
        let cv = cascade_doc("", "p", Some("line-height: 1.5"));
        assert_eq!(cv.line_height, ComputedLineHeight::Number(1.5));
    }

    #[test]
    fn line_height_is_inherited_child_carries_parent_number() {
        // spec §5.1 "Inheritance: Yes"。<p style="line-height: 1.5"> の子 <span>
        // は自身 rule 無しでも parent の LineHeight::Number(1.5) を継承する
        // (unitless number の specified-value inherit special behavior は
        // cascade static side では raw value 継承として観測される)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("line-height: 1.5"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].line_height, ComputedLineHeight::Number(1.5));
        assert_eq!(
            r.computed[span].line_height,
            ComputedLineHeight::Number(1.5),
            "line-height must be inherited (§5.1 Yes)"
        );
    }

    #[test]
    fn counter_reset_wired_through_cascade_from_inline_style() {
        // <div style="counter-reset: chapter"> → ComputedValues.counter_reset
        // に `[("chapter", 0)]` が届く。parser → PropertyValue → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。
        // counter_reset は Arc<Vec<..>>、`*cv.counter_reset` で deref-compare。
        let cv = cascade_doc("", "div", Some("counter-reset: chapter"));
        assert_eq!(*cv.counter_reset, vec![(SmolStr::new("chapter"), 0)]);
        // 他 counter property は non-inherited の initial (empty) のまま
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
    }

    #[test]
    fn quotes_wired_through_cascade_from_inline_style() {
        // <p style='quotes: "«" "»"'> → ComputedValues.quotes に `[("«", "»")]`
        // が届く。parser → PropertyValue::Quotes → apply_value →
        // ComputedValues の end-to-end 疎通 smoke (`counter_reset` wire-through
        // pattern を踏襲)。quotes は Arc<Vec<..>>、`*cv.quotes` で deref-compare。
        let cv = cascade_doc("", "p", Some(r#"quotes: "«" "»""#));
        assert_eq!(*cv.quotes, vec![(SmolStr::new("«"), SmolStr::new("»"))]);
    }

    #[test]
    fn quotes_is_inherited_child_carries_parent_pairs() {
        // CSS Content 3 §2.4.1 "Inherited: yes"。<div style='quotes: ...'> の
        // 子 <span> は自身 rule 無しでも parent の quotes pairs を継承する
        // (`line_height_is_inherited_child_carries_parent_number` と同型 —
        // counter-* (non-inherited) と対照的に、こちらは real cascade tree を
        // 組んで inheritance walk 自体を通す)。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some(r#"quotes: "«" "»" "‹" "›""#));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let expected = vec![
            (SmolStr::new("«"), SmolStr::new("»")),
            (SmolStr::new("‹"), SmolStr::new("›")),
        ];
        assert_eq!(*r.computed[div].quotes, expected);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            *r.computed[span].quotes, expected,
            "quotes must be inherited (CSS Content 3 §2.4.1 Inherited: yes)"
        );
    }

    #[test]
    fn content_wired_through_cascade_from_inline_style() {
        // <p style='content: "hello"'> → ComputedValues.content に
        // `[Literal("hello")]` が届く。parser → PropertyValue::Content →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // counter-* wire-through pattern を踏襲。
        // content は Arc<Vec<..>>、`*cv.content` で deref-compare。
        // Literal は SmolStr payload (owned String → SmolStr conversion)。
        use crate::property::ContentComponent;
        use smol_str::SmolStr;
        let cv = cascade_doc("", "p", Some(r#"content: "hello""#));
        assert_eq!(
            *cv.content,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn string_set_wired_through_cascade_from_inline_style() {
        // <p style='string-set: chapter_title "hello"'> → ComputedValues.string_set
        // に `[(chapter_title, [Literal("hello")])]` が届く。
        // parser → PropertyValue::StringSet → apply_value → ComputedValues の
        // end-to-end 疎通 smoke。counter-* / content wire-through pattern を踏襲。
        // string_set は Arc<Vec<..>>、Literal は SmolStr。indexing +
        // field access は Arc<Vec<T>> の Deref chain (`&[T]`) 経由でそのまま
        // 通る (dom/paint consumer 波及 0)。
        use crate::property::ContentComponent;
        let cv = cascade_doc("", "p", Some(r#"string-set: chapter_title "hello""#));
        assert_eq!(cv.string_set.len(), 1);
        assert_eq!(cv.string_set[0].0, SmolStr::new("chapter_title"));
        assert_eq!(
            cv.string_set[0].1,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn string_set_is_non_inherited_child_starts_from_initial_empty() {
        // CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>:
        // string-set は non-inherited。<p style='string-set: a "x"'>
        // の子 <span> は自身 rule がなく、string_set は initial (empty Vec)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some(r#"string-set: a "x""#));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].string_set.len(),
            1,
            "parent should carry its own string-set"
        );
        assert!(
            r.computed[span].string_set.is_empty(),
            "child should not inherit string-set"
        );
    }

    #[test]
    fn content_is_non_inherited_child_starts_from_initial_empty() {
        // spec §2.1: content は non-inherited。<p style="content: 'x'">
        // の子 <span> は自身 rule がなく、content は initial (empty Vec)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some(r#"content: "parent""#));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].content.len(),
            1,
            "parent should carry its own content"
        );
        assert!(
            r.computed[span].content.is_empty(),
            "child should not inherit content"
        );
    }

    #[test]
    fn list_style_values_inherit_and_author_override() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(
            0,
            "ol",
            Some("list-style-type: upper-roman; list-style-position: inside"),
        );
        let child = doc.push_element(parent, "li", None);
        let override_child = doc.push_element(
            parent,
            "li",
            Some("list-style-type: none; list-style-position: outside"),
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[parent].list_style_type,
            ListStyleType::Named("upper-roman".into())
        );
        assert_eq!(
            r.computed[child].list_style_type,
            ListStyleType::Named("upper-roman".into())
        );
        assert_eq!(
            r.computed[child].list_style_position,
            ListStylePosition::Inside
        );
        assert_eq!(
            r.computed[override_child].list_style_type,
            ListStyleType::None
        );
        assert_eq!(
            r.computed[override_child].list_style_position,
            ListStylePosition::Outside
        );
    }

    #[test]
    fn marker_selector_populates_pseudo_map_and_inherits_list_style() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            r##"li { display: list-item; list-style-type: decimal } li::marker { content: "#"; color: blue }"##,
        );
        let li = doc.push_element(0, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let marker = r
            .pseudo
            .get(&(StyleNodeId(li as u64), PseudoElem::Marker))
            .expect("::marker entry must exist");
        assert_eq!(marker.color, BLUE);
        assert_eq!(*marker.content, vec![ContentComponent::Literal("#".into())]);
        assert_eq!(
            marker.list_style_type,
            ListStyleType::Named("decimal".into())
        );
        assert_eq!(marker.list_style_position, ListStylePosition::Outside);
    }

    #[test]
    fn before_selector_populates_pseudo_map_with_content() {
        // `.foo::before { content: "x" }` on `<p class="foo">` — the real
        // element's own `computed` must NOT carry `content` (that selector
        // never matches `p` itself, only its `::before`), while
        // `result.pseudo[&(p, PseudoElem::Before)]` must.
        use crate::property::ContentComponent;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#".foo::before { content: "x" }"#);
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            r.computed[p].content.is_empty(),
            "the ::before rule must not leak onto the real element itself"
        );
        let before = r
            .pseudo
            .get(&(StyleNodeId(p as u64), PseudoElem::Before))
            .expect("::before entry must exist");
        assert_eq!(*before.content, vec![ContentComponent::Literal("x".into())]);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !r.pseudo
                .contains_key(&(StyleNodeId(p as u64), PseudoElem::After)),
            "no ::after rule matched — no entry"
        );
    }

    #[test]
    fn after_selector_populates_pseudo_map_independently_of_before() {
        use crate::property::ContentComponent;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p::before { content: "b" } p::after { content: "a" }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        let after = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::After)];
        assert_eq!(*before.content, vec![ContentComponent::Literal("b".into())]);
        assert_eq!(*after.content, vec![ContentComponent::Literal("a".into())]);
    }

    #[test]
    fn pseudo_element_computed_values_inherit_from_real_element_not_initial() {
        // CSS Pseudo-Elements Module Level 4 §4
        // <https://drafts.csswg.org/css-pseudo-4/#treelike>: "They inherit
        // any inheritable properties from their originating element" — so
        // `color` (inherited) on `::before` must come from the real
        // element's own computed color, not the document's initial black,
        // when the `::before` rule itself sets nothing for `color`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p { color: red } p::before { content: "x" }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            before.color, RED,
            "::before must inherit color from its real originating element"
        );
    }

    #[test]
    fn pseudo_element_own_declaration_overrides_inherited_value() {
        // A `::before` rule can still set its own `color`, overriding what
        // it would otherwise inherit from the real element.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            r#"p { color: red } p::before { content: "x"; color: blue }"#,
        );
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(before.color, BLUE);
    }

    #[test]
    fn pseudo_element_custom_property_wired_through_cascade() {
        // Exercises `CascadedArena`'s custom-property counterpart of the
        // pseudo arena (`pseudo_custom_decls`/`pseudo_custom_ranges`,
        // `collect_cascaded`'s `pseudo_before_custom`/`pseudo_after_custom`
        // scratch buffers) — every other pseudo-element test above only
        // ever pushes through the plain-property side
        // (`pseudo_decls`/`pseudo_ranges`). A `::before` rule can declare a
        // custom property exactly like a real element can; this pins that
        // it actually reaches the pseudo's own `custom_properties`
        // environment (via `resolve_custom_properties` in
        // `resolve_inheritance`'s pseudo-element section), not just the
        // parent's inherited one.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p::before { content: "x"; --accent: red }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(before.custom_properties.get("--accent"), Some("red".into()));
    }

    #[test]
    fn pseudo_element_entry_exists_with_empty_content_when_rule_sets_no_content() {
        // `CascadeResult::pseudo` doc's contract to the consumer: map
        // presence only means "some ::before/::after rule matched" — it says
        // nothing about whether that rule actually set `content`. A
        // `::before` rule that sets only `color` (forgets `content`, an easy
        // authoring mistake) must still produce an entry, with an empty
        // `content` list (this crate's `normal`/`none` representation) —
        // not a missing entry, and not a panic.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p::before { color: blue }");
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(before.color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            before.content.is_empty(),
            "no `content` declaration on the rule — must compute to the \
             empty (normal/none) representation, not panic or default to \
             something else"
        );
    }

    #[test]
    fn running_template_wired_through_cascade_from_inline_style() {
        // <div style="position: running(header)"> → ComputedValues.running_templates
        // に `[RunningTemplate{name:"header"}]` が届く。parser → PropertyValue::Position
        // → apply_value → ComputedValues の end-to-end 疎通 smoke。
        // counter-* / content / string-set wire-through pattern を踏襲。
        use crate::computed::RunningTemplate;
        let cv = cascade_doc("", "div", Some("position: running(header)"));
        assert_eq!(
            cv.running_templates,
            vec![RunningTemplate {
                name: SmolStr::new("header")
            }]
        );
    }

    #[test]
    fn running_template_is_non_inherited_child_starts_from_initial_empty() {
        // CSS GCPM 3 §1.2.1。position が non-inherited であることは CSS
        // Positioned Layout 3 §2 <https://www.w3.org/TR/css-position-3/#position-property>
        // の propdef "Inherited: no"。<div style="position: running(hdr)"> の
        // 子 <span> は自身の rule がなく running_templates は initial (empty)。
        // sibling: string_set / content non-inherited と同じ shape。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("position: running(hdr)"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].running_templates.len(),
            1,
            "parent should carry its own running_templates seed"
        );
        assert!(
            r.computed[span].running_templates.is_empty(),
            "child should not inherit running_templates"
        );
    }

    #[test]
    fn position_static_yields_empty_running_templates() {
        // position: static (spec baseline) の場合 apply_value は no-op、
        // running_templates は initial の空 Vec が残る。標準 pattern の pin。
        let cv = cascade_doc("", "div", Some("position: static"));
        assert!(cv.running_templates.is_empty());
    }

    #[test]
    fn static_position_wins_over_running_via_source_order() {
        // `Static` variant の load-bearing 検証。
        // 同一 declaration block 内で `position: running(hdr); position: static`
        // → CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> で後方
        // declaration が同 rank/spec/order で勝つ (source_order
        // が同じでも `beats` の `>=` で最後の候補が上書きする)。winner は
        // Position(Static)、apply_value は no-op → running_templates 空。
        let cv = cascade_doc("", "div", Some("position: running(hdr); position: static"));
        assert!(
            cv.running_templates.is_empty(),
            "later `position: static` must suppress earlier `running(hdr)` — \
             running_templates should stay empty when Static wins the cascade"
        );
    }

    #[test]
    fn text_justify_wired_through_cascade_from_inline_style() {
        // <p style="text-justify: inter-word"> → ComputedValues.text_justify。
        // 上記 text-align pattern を踏襲。
        use crate::property::TextJustify;
        let cv = cascade_doc("", "p", Some("text-justify: inter-word"));
        assert_eq!(cv.text_justify, TextJustify::InterWord);
    }

    #[test]
    fn text_justify_distribute_parses() {
        // legacy `distribute` を受理する (WPT text-justify-distribute-001)。
        use crate::property::TextJustify;
        let cv = cascade_doc("", "p", Some("text-justify: distribute"));
        assert_eq!(cv.text_justify, TextJustify::Distribute);
    }

    #[test]
    fn text_justify_inherits_from_parent_element() {
        // CSS Text 3 §6.2: text-justify は **inherited**。
        use crate::property::TextJustify;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-justify: none"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_justify, TextJustify::None);
        assert_eq!(r.computed[span].text_justify, TextJustify::None);
    }

    #[test]
    fn text_align_last_wired_through_cascade_from_inline_style() {
        // <p style="text-align-last: justify"> → ComputedValues.text_align_last。
        use crate::property::TextAlignLast;
        let cv = cascade_doc("", "p", Some("text-align-last: justify"));
        assert_eq!(cv.text_align_last, TextAlignLast::Justify);
    }

    #[test]
    fn text_align_last_inherits_from_parent_element() {
        // CSS Text 3 §6.1: text-align-last は **inherited**。
        use crate::property::TextAlignLast;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align-last: center"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_align_last, TextAlignLast::Center);
        assert_eq!(r.computed[span].text_align_last, TextAlignLast::Center);
    }

    #[test]
    fn text_wrap_nowrap_wired_through_cascade_from_inline_style() {
        use crate::property::TextWrapMode;
        let cv = cascade_doc("", "p", Some("text-wrap: nowrap"));
        assert_eq!(cv.text_wrap, TextWrapMode::Nowrap);
    }

    #[test]
    fn text_wrap_wrap_is_default_and_inherited() {
        use crate::property::TextWrapMode;
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.text_wrap, TextWrapMode::Wrap);
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-wrap: nowrap"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_wrap, TextWrapMode::Nowrap);
        assert_eq!(r.computed[span].text_wrap, TextWrapMode::Nowrap);
    }

    #[test]
    fn text_align_wired_through_cascade_from_inline_style() {
        // <p style="text-align: center"> → ComputedValues.text_align に
        // TextAlign::Center が届く。parser → PropertyValue::TextAlign →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // counter-* / content / string-set / position wire-through
        // pattern を踏襲 (原則 1 前例主義)。
        use crate::property::TextAlign;
        let cv = cascade_doc("", "p", Some("text-align: center"));
        assert_eq!(cv.text_align, TextAlign::Center);
    }

    #[test]
    fn text_align_inherits_from_parent_element() {
        // CSS Text 3 §6.1: text-align は **inherited** (color と同じ handling)。
        // <p style="text-align: center"> の子 <span> は自身 rule 無しでも
        // 親の text_align (Center) を引き継ぐ。inheritance walk が
        // inherit_from 経由で text_align を copy することを pin。
        //
        // Verification #7 の中核 assertion (parent center → child Center を確認)。
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align: center"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_align, TextAlign::Center);
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Center,
            "child should inherit text-align from parent (CSS Text 3 §6.1 inherited property)"
        );
    }

    #[test]
    fn text_align_inheritance_contrasts_with_display_non_inheritance() {
        // Verification #7 (contrast): text-align (inherited) と display
        // (non-inherited) を同一 fixture で対比 — inheritance discipline を明示。
        // parent が両 property を持ち、child は inherit で text-align のみ引き継ぐ、
        // display は initial (Inline) に落ちる。inherit_from の inherited /
        // non-inherited 分類が正しく機能していることの pin。
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        // p に UA-like rule として display: block を Author 側で置く (現状
        // UA rule も同 rank に居るので、child が inherit しない性質だけを見る)
        doc.push_text(s, "p { display: block; text-align: right }");
        let p = doc.push_element(0, "p", None);
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // parent: 両 property が Author rule で set される。
        assert_eq!(r.computed[p].display, DisplayValue::Block);
        assert_eq!(r.computed[p].text_align, TextAlign::Right);
        // child: 自身 rule 無し。text-align (inherited) は Right を引き継ぐが、
        // display (non-inherited) は initial (Inline) に落ちる。
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Right,
            "text-align must inherit (CSS Text 3 §6.1 inherited)"
        );
        assert_eq!(
            r.computed[span].display,
            DisplayValue::Inline,
            "display must NOT inherit (CSS Display 3 §2 Inherited: no) — initial Inline"
        );
    }

    #[test]
    fn text_indent_wired_through_cascade_from_inline_style() {
        // <p style="text-indent: 20px"> → ComputedValues.text_indent に
        // ComputedLengthPercentage::Px(20.0) が届く。parser →
        // PropertyValue::TextIndent → apply_value → ComputedValues の
        // end-to-end 疎通 smoke (`text_align_wired_through_cascade_from_inline_style`
        // と同 pattern)。
        let cv = cascade_doc("", "p", Some("text-indent: 20px"));
        assert_eq!(cv.text_indent, ComputedLengthPercentage::Px(20.0));
    }

    #[test]
    fn text_indent_flags_wired_through_cascade_from_inline_style() {
        // <p style="text-indent: 2em hanging each-line"> → length plus flags.
        let cv = cascade_doc("", "p", Some("text-indent: 2em hanging each-line"));
        assert!(cv.text_indent_hanging);
        assert!(cv.text_indent_each_line);
    }

    #[test]
    fn text_indent_percentage_stays_unresolved_in_computed_layer() {
        // CSS Text 3 §8.1 "Computed value: computed <length-percentage>
        // value, plus any specified keywords" — `%` は block container 自身の
        // inline-axis inner size 依存 (used value 層) なので、この crate の
        // computed 層では `Percent` のまま残る (`padding` / `width` と同じ
        // 扱い、`ComputedValues::padding` doc 参照)。
        let cv = cascade_doc("", "p", Some("text-indent: 10%"));
        assert_eq!(cv.text_indent, ComputedLengthPercentage::Percent(10.0));
    }

    #[test]
    fn text_indent_inherits_from_parent_element() {
        // CSS Text 3 §8.1: text-indent は **inherited**。<p> の `2em` は親の
        // font-size (20px) 基準で 40px に絶対化され、子 <span> はその**絶対化
        // 済み 40px を再解決せず継承**する (`lift_line_height` doc が
        // line-height について説明する挙動と同型 — 子が独自の font-size
        // (10px) を持っていても 40px のままであることで、この
        // "re-resolve しない" 性質を子の font-size を変えて check する)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-size: 20px; text-indent: 2em"));
        let span = doc.push_element(p, "span", Some("font-size: 10px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].text_indent,
            ComputedLengthPercentage::Px(40.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_indent,
            ComputedLengthPercentage::Px(40.0),
            "child should inherit text-indent's already-absolutized 40px \
             unchanged (CSS Text 3 §8.1 inherited property), not re-resolve \
             `2em` against its own 10px font-size"
        );
    }

    #[test]
    fn text_indent_ch_preserves_source_font_through_inheritance_and_override() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-size: 20px; text-indent: 1ch"));
        let inherited = doc.push_element(p, "span", Some("font-size: 40px"));
        let own = doc.push_element(p, "strong", Some("font-size: 40px; text-indent: 2ch"));
        let own_child = doc.push_element(own, "i", Some("font-size: 10px"));
        let cleared = doc.push_element(p, "em", Some("font-size: 40px; text-indent: 2em"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(r.computed[p].text_indent_ch_factor, Some(1.0));
        assert_eq!(r.computed[inherited].text_indent_ch_factor, Some(1.0));
        assert_eq!(
            r.computed[inherited]
                .text_indent_ch_font
                .as_ref()
                .expect("inherited source font")
                .size,
            ComputedLength(20.0)
        );
        assert_eq!(r.computed[own].text_indent_ch_factor, Some(2.0));
        assert_eq!(
            r.computed[own]
                .text_indent_ch_font
                .as_ref()
                .expect("own source font")
                .size,
            ComputedLength(40.0)
        );
        assert_eq!(
            r.computed[own_child]
                .text_indent_ch_font
                .as_ref()
                .expect("inherited own source font")
                .size,
            ComputedLength(40.0)
        );
        assert_eq!(r.computed[cleared].text_indent_ch_factor, None);
        assert_eq!(r.computed[cleared].text_indent_ch_font, None);
    }

    #[test]
    fn text_indent_inheritance_contrasts_with_padding_non_inheritance() {
        // Verification (contrast): text-indent (inherited) と padding-top
        // (non-inherited) を同一 fixture で対比 —
        // `text_align_inheritance_contrasts_with_display_non_inheritance` と
        // 同 pattern。child は自身 rule 無し、text-indent のみ引き継ぎ、
        // padding-top は initial (0) に落ちる。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-indent: 15px; padding-top: 15px"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].text_indent,
            ComputedLengthPercentage::Px(15.0)
        );
        assert_eq!(
            r.computed[p].padding.top,
            ComputedLengthPercentage::Px(15.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_indent,
            ComputedLengthPercentage::Px(15.0),
            "text-indent must inherit (CSS Text 3 §8.1 Inherited: yes)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].padding.top,
            ComputedLengthPercentage::Px(0.0),
            "padding-top must NOT inherit (CSS Box 3 §4.1 Inherited: no) — initial 0"
        );
    }

    #[test]
    fn text_align_child_own_value_wins_over_inherited() {
        // parent center + child left → child は自身 rule の Left が cascade winner。
        // inheritance は「rule 無し fallback」であって override 元ではないことを pin。
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align: center"));
        let span = doc.push_element(p, "span", Some("text-align: left"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_align, TextAlign::Center);
        assert_eq!(r.computed[span].text_align, TextAlign::Left);
    }

    #[test]
    fn font_style_wired_through_cascade_from_inline_style() {
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: italic"));
        assert_eq!(cv.font_style, FontStyle::Italic);
    }

    #[test]
    fn font_style_oblique_wired_through_cascade_from_inline_style() {
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: oblique"));
        assert_eq!(cv.font_style, FontStyle::Oblique);
    }

    #[test]
    fn font_style_inherits_from_parent_element() {
        // CSS Fonts 4 §2.4: font-style は **inherited**.
        use crate::property::FontStyle;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-style: italic"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_style, FontStyle::Italic);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].font_style,
            FontStyle::Italic,
            "child should inherit font-style from parent (CSS Fonts 4 §2.4 Inherited: yes)"
        );
    }

    #[test]
    fn font_style_child_own_value_wins_over_inherited() {
        use crate::property::FontStyle;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-style: italic"));
        let span = doc.push_element(p, "span", Some("font-style: normal"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_style, FontStyle::Italic);
        assert_eq!(r.computed[span].font_style, FontStyle::Normal);
    }

    #[test]
    fn font_style_scope_cut_left_is_dropped_and_prior_wins_via_stylesheet() {
        // `left` / `right` are spec-valid `font-style` keywords (CSS Fonts 4
        // §2.4) but this crate's scope excludes them (`FontStyle` doc).
        // Second rule is parse-dropped, so first rule stays winner.
        use crate::property::FontStyle;
        let cv = cascade_doc("p { font-style: italic } p { font-style: left }", "p", None);
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            cv.font_style,
            FontStyle::Italic,
            "scope-limited `font-style: left` is dropped, prior `italic` must remain winner (current crate behaviour)"
        );
    }

    #[test]
    fn font_style_scope_cut_right_is_dropped_and_prior_wins_via_inline() {
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: italic; font-style: right"));
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            cv.font_style,
            FontStyle::Italic,
            "scope-limited `font-style: right` is dropped, prior `italic` must remain winner"
        );
    }

    #[test]
    fn font_style_scope_cut_oblique_angle_is_dropped_and_prior_wins() {
        // `oblique <angle>` is spec-valid (CSS Fonts 4 §2.4) but this crate
        // accepts only bare `oblique` (`FontStyle` doc's Scope carving).
        // `oblique 14deg` is consumed as `oblique` then rejected by
        // `DeclParser::expect_exhausted` for the leftover `<angle>` token,
        // so the whole declaration is dropped — prior wins.
        use crate::property::FontStyle;
        let cv = cascade_doc(
            "",
            "p",
            Some("font-style: italic; font-style: oblique 14deg"),
        );
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            cv.font_style,
            FontStyle::Italic,
            "scope-limited `font-style: oblique 14deg` is dropped, prior `italic` must remain winner"
        );
        // Same via stylesheet ordering.
        let cv = cascade_doc(
            "p { font-style: italic } p { font-style: oblique 14deg }",
            "p",
            None,
        );
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(cv.font_style, FontStyle::Italic);
    }

    #[test]
    fn font_style_scope_cut_alone_falls_back_to_initial() {
        // A lone scope-limited declaration never wins, so the property stays at
        // its initial value (`normal`), not the scope-limited keyword.
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: left"));
        assert_eq!(cv.font_style, FontStyle::Normal);
        let cv = cascade_doc("", "p", Some("font-style: oblique 14deg"));
        assert_eq!(cv.font_style, FontStyle::Normal);
    }

    #[test]
    fn font_style_bare_oblique_is_not_scope_cut_and_wins() {
        // Control: bare `oblique` IS implemented and must win over a prior
        // declaration, proving the prior-wins above is due to the scope-limited
        // drop, not a generic cascade bug.
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: italic; font-style: oblique"));
        assert_eq!(cv.font_style, FontStyle::Oblique);
        let cv = cascade_doc(
            "p { font-style: italic } p { font-style: oblique }",
            "p",
            None,
        );
        assert_eq!(cv.font_style, FontStyle::Oblique);
    }

    #[test]
    fn font_variant_caps_wired_through_cascade_from_inline_style() {
        use crate::property::FontVariantCaps;
        let cv = cascade_doc("", "p", Some("font-variant-caps: small-caps"));
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
    }

    #[test]
    fn font_variant_caps_inherits_from_parent_element() {
        // CSS Fonts Module Level 3 §6.6: font-variant-caps は **inherited**.
        use crate::property::FontVariantCaps;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-variant-caps: small-caps"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_variant_caps, FontVariantCaps::SmallCaps);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].font_variant_caps,
            FontVariantCaps::SmallCaps,
            "child should inherit font-variant-caps from parent (CSS Fonts Module Level 3 §6.6 Inherited: yes)"
        );
    }

    #[test]
    fn font_variant_caps_child_own_value_wins_over_inherited() {
        use crate::property::FontVariantCaps;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-variant-caps: small-caps"));
        let span = doc.push_element(p, "span", Some("font-variant-caps: normal"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(r.computed[span].font_variant_caps, FontVariantCaps::Normal);
    }

    #[test]
    fn text_transform_wired_through_cascade_from_inline_style() {
        use crate::property::TextTransform;
        let cv = cascade_doc("", "p", Some("text-transform: uppercase"));
        assert_eq!(cv.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn text_transform_inherits_from_parent_element() {
        // CSS Text Module Level 3 §2.1: text-transform は **inherited**.
        use crate::property::TextTransform;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-transform: uppercase"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_transform, TextTransform::Uppercase);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_transform,
            TextTransform::Uppercase,
            "child should inherit text-transform from parent (CSS Text Module Level 3 §2.1 Inherited: yes)"
        );
    }

    #[test]
    fn visibility_wired_through_cascade_from_inline_style() {
        use crate::property::Visibility;
        let cv = cascade_doc("", "p", Some("visibility: hidden"));
        assert_eq!(cv.visibility, Visibility::Hidden);
    }

    #[test]
    fn visibility_inherits_from_parent_element() {
        // CSS Display 3 §4: visibility は **inherited**.
        use crate::property::Visibility;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("visibility: hidden"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].visibility, Visibility::Hidden);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].visibility,
            Visibility::Hidden,
            "child should inherit visibility from parent (CSS Display 3 §4 Inherited: yes)"
        );
    }

    #[test]
    fn table_layout_wired_through_cascade_from_inline_style() {
        use crate::property::TableLayoutValue;
        let cv = cascade_doc("", "table", Some("table-layout: fixed"));
        assert_eq!(cv.table_layout, TableLayoutValue::Fixed);
    }

    #[test]
    fn table_layout_does_not_inherit_from_parent_element() {
        // CSS Tables 3 §4: table-layout は **non-inherited**.
        use crate::property::TableLayoutValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("table-layout: fixed"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[table].table_layout, TableLayoutValue::Fixed);
        assert_eq!(
            r.computed[td].table_layout,
            TableLayoutValue::Auto,
            "child should reset table-layout to initial (CSS Tables 3 §4 Inherited: no)"
        );
    }

    #[test]
    fn border_collapse_wired_through_cascade_from_inline_style() {
        use crate::property::BorderCollapseValue;
        let cv = cascade_doc("", "table", Some("border-collapse: collapse"));
        assert_eq!(cv.border_collapse, BorderCollapseValue::Collapse);
    }

    #[test]
    fn border_collapse_inherits_from_parent_element() {
        // CSS Tables 3 §6: border-collapse は **inherited**.
        use crate::property::BorderCollapseValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("border-collapse: collapse"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[table].border_collapse,
            BorderCollapseValue::Collapse
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[td].border_collapse,
            BorderCollapseValue::Collapse,
            "child should inherit border-collapse from parent (CSS Tables 3 §6 Inherited: yes)"
        );
    }

    #[test]
    fn border_spacing_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "table", Some("border-spacing: 10px 20px"));
        assert_eq!(
            cv.border_spacing.horizontal,
            crate::resolve::ComputedLength(10.0)
        );
        assert_eq!(
            cv.border_spacing.vertical,
            crate::resolve::ComputedLength(20.0)
        );
        // WPT computed: `"10px 20px"` stays two lengths.
        assert_eq!(cv.border_spacing.serialized(), "10px 20px");
    }

    #[test]
    fn border_spacing_zero_serializes_shortest() {
        // WPT computed: `"0"` → `"0px"` (not `"0px 0px"`, CSSOM §2.1
        // shortest serialization).
        let cv = cascade_doc("", "table", Some("border-spacing: 0"));
        assert_eq!(cv.border_spacing.serialized(), "0px");
    }

    #[test]
    fn border_spacing_single_value_doubles_to_both_axes() {
        let cv = cascade_doc("", "table", Some("border-spacing: 10px"));
        assert_eq!(cv.border_spacing.serialized(), "10px");
    }

    #[test]
    fn border_spacing_resolves_em_against_own_font_size() {
        // font-size 40px の node での `0.5em` → 20px (WPT computed file が
        // `#target` に `font-size: 40px` を指定するのと同じ基準)。
        let cv = cascade_doc(
            "",
            "table",
            Some("font-size: 40px; border-spacing: 0.5em 10px"),
        );
        assert_eq!(cv.border_spacing.serialized(), "20px 10px");
    }

    #[test]
    fn border_spacing_inherits_from_parent_element() {
        // CSS Tables 3 §6.1: border-spacing は **inherited**.
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("border-spacing: 10px 20px"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[td].border_spacing.serialized(),
            "10px 20px",
            "child should inherit border-spacing from parent (CSS Tables 3 §6.1 Inherited: yes)"
        );
    }

    #[test]
    fn caption_side_wired_through_cascade_from_inline_style() {
        use crate::property::CaptionSideValue;
        let cv = cascade_doc("", "table", Some("caption-side: bottom"));
        assert_eq!(cv.caption_side, CaptionSideValue::Bottom);
        // WPT caption-side-computed.html: single keyword serializes as-is.
        let cv_top = cascade_doc("", "table", Some("caption-side: top"));
        assert_eq!(cv_top.caption_side, CaptionSideValue::Top);
    }

    #[test]
    fn caption_side_inherits_from_parent_element() {
        // CSS Tables 3 §7: caption-side は **inherited**.
        use crate::property::CaptionSideValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("caption-side: bottom"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[td].caption_side,
            CaptionSideValue::Bottom,
            "child should inherit caption-side from parent (CSS Tables 3 §7 Inherited: yes)"
        );
    }

    #[test]
    fn empty_cells_wired_through_cascade_from_inline_style() {
        use crate::property::EmptyCellsValue;
        let cv = cascade_doc("", "table", Some("empty-cells: hide"));
        assert_eq!(cv.empty_cells, EmptyCellsValue::Hide);
        // WPT empty-cells-computed.html: single keyword serializes as-is.
        let cv_show = cascade_doc("", "table", Some("empty-cells: show"));
        assert_eq!(cv_show.empty_cells, EmptyCellsValue::Show);
    }

    #[test]
    fn empty_cells_inherits_from_parent_element() {
        // CSS Tables 3 §8: empty-cells は **inherited**.
        use crate::property::EmptyCellsValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("empty-cells: hide"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[td].empty_cells,
            EmptyCellsValue::Hide,
            "child should inherit empty-cells from parent (CSS Tables 3 §8 Inherited: yes)"
        );
    }

    #[test]
    fn word_break_wired_through_cascade_from_inline_style() {
        use crate::property::WordBreak;
        let cv = cascade_doc("", "p", Some("word-break: break-all"));
        assert_eq!(cv.word_break, WordBreak::BreakAll);
    }

    #[test]
    fn word_break_inherits_from_parent_element() {
        // CSS Text 3 §5.1: word-break は **inherited**.
        use crate::property::WordBreak;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("word-break: break-all"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].word_break, WordBreak::BreakAll);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].word_break,
            WordBreak::BreakAll,
            "child should inherit word-break from parent (CSS Text 3 §5.1 Inherited: yes)"
        );
    }

    #[test]
    fn text_transform_child_own_value_wins_over_inherited() {
        use crate::property::TextTransform;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-transform: uppercase"));
        let span = doc.push_element(p, "span", Some("text-transform: lowercase"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_transform, TextTransform::Uppercase);
        assert_eq!(r.computed[span].text_transform, TextTransform::Lowercase);
    }

    #[test]
    fn visibility_child_own_value_wins_over_inherited() {
        use crate::property::Visibility;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("visibility: hidden"));
        let span = doc.push_element(p, "span", Some("visibility: visible"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].visibility, Visibility::Hidden);
        assert_eq!(r.computed[span].visibility, Visibility::Visible);
    }

    #[test]
    fn word_break_child_own_value_wins_over_inherited() {
        use crate::property::WordBreak;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("word-break: break-all"));
        let span = doc.push_element(p, "span", Some("word-break: keep-all"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].word_break, WordBreak::BreakAll);
        assert_eq!(r.computed[span].word_break, WordBreak::KeepAll);
    }

    #[test]
    fn word_break_break_word_is_now_accepted_and_wins_via_stylesheet() {
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "p { word-break: break-all } p { word-break: break-word }",
            "p",
            None,
        );
        assert_eq!(
            cv.word_break,
            WordBreak::BreakWord,
            "break-word is now implemented and must win over break-all"
        );
    }

    #[test]
    fn word_break_break_word_is_now_accepted_and_wins_via_inline() {
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "",
            "p",
            Some("word-break: break-all; word-break: break-word"),
        );
        assert_eq!(
            cv.word_break,
            WordBreak::BreakWord,
            "break-word is now implemented and must win over break-all"
        );
    }

    #[test]
    fn word_break_break_word_wins_over_keep_all_in_same_rule() {
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "p { word-break: keep-all; word-break: break-word }",
            "p",
            None,
        );
        assert_eq!(cv.word_break, WordBreak::BreakWord);
    }

    #[test]
    fn word_break_break_word_alone_is_accepted() {
        use crate::property::WordBreak;
        let cv = cascade_doc("", "p", Some("word-break: break-word"));
        assert_eq!(cv.word_break, WordBreak::BreakWord);
        let cv = cascade_doc("p { word-break: break-word }", "p", None);
        assert_eq!(cv.word_break, WordBreak::BreakWord);
    }

    #[test]
    fn word_break_valid_later_overrides_prior_scope_cut_alone() {
        // Control: `break-word` being dropped must not poison a later valid
        // declaration in the same element. `break-word` (dropped) then
        // `break-all` (valid) → `break-all` wins, proving the drop is
        // per-declaration, not per-property.
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "",
            "p",
            Some("word-break: break-word; word-break: break-all"),
        );
        assert_eq!(cv.word_break, WordBreak::BreakAll);
    }

    #[test]
    fn overflow_wrap_wired_through_cascade_from_inline_style() {
        use crate::property::OverflowWrap;
        let cv = cascade_doc("", "p", Some("overflow-wrap: anywhere"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::Anywhere);
    }

    #[test]
    fn word_wrap_legacy_alias_wired_through_cascade_same_as_overflow_wrap() {
        // CSS Text 3 §5.4 verbatim: "For legacy reasons, UAs must treat
        // word-wrap as a legacy name alias of the overflow-wrap property."
        use crate::property::OverflowWrap;
        let cv = cascade_doc("", "p", Some("word-wrap: break-word"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::BreakWord);
    }

    #[test]
    fn word_wrap_and_overflow_wrap_cascade_against_each_other_as_one_property() {
        // `OverflowWrap` doc's "legacy alias" section: the two names share
        // one `PropertyKey`, so — unlike two genuinely different properties
        // — a later declaration under either name overrides an earlier
        // declaration under the *other* name (CSS Cascading L4 §6.1 "Order
        // of Appearance": "The last declaration in document order wins.",
        // same rule pinned for a single property name by the
        // `later_duplicate_in_inline_wins` sibling test above).
        use crate::property::OverflowWrap;
        let cv = cascade_doc("", "p", Some("overflow-wrap: normal; word-wrap: anywhere"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::Anywhere);
        let cv = cascade_doc("", "p", Some("word-wrap: anywhere; overflow-wrap: normal"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::Normal);
    }

    #[test]
    fn overflow_wrap_inherits_from_parent_element() {
        // CSS Text 3 §5.4: overflow-wrap は **inherited**.
        use crate::property::OverflowWrap;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("overflow-wrap: anywhere"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].overflow_wrap, OverflowWrap::Anywhere);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].overflow_wrap,
            OverflowWrap::Anywhere,
            "child should inherit overflow-wrap from parent (CSS Text 3 §5.4 Inherited: yes)"
        );
    }

    #[test]
    fn overflow_wrap_child_own_value_wins_over_inherited() {
        use crate::property::OverflowWrap;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("overflow-wrap: anywhere"));
        let span = doc.push_element(p, "span", Some("overflow-wrap: normal"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].overflow_wrap, OverflowWrap::Anywhere);
        assert_eq!(r.computed[span].overflow_wrap, OverflowWrap::Normal);
    }

    #[test]
    fn white_space_break_spaces_is_now_accepted_and_wins() {
        use crate::property::WhiteSpace;
        let cv = cascade_doc(
            "",
            "p",
            Some("white-space: pre-wrap; white-space: break-spaces"),
        );
        assert_eq!(
            cv.white_space,
            WhiteSpace::BreakSpaces,
            "break-spaces is now implemented and must win over pre-wrap"
        );
        let cv = cascade_doc("", "p", Some("white-space: break-spaces"));
        assert_eq!(cv.white_space, WhiteSpace::BreakSpaces);
    }

    #[test]
    fn vertical_align_top_and_bottom_cascade_as_computed_values() {
        use crate::property::VerticalAlign;
        let cv = cascade_doc(
            "",
            "span",
            Some("vertical-align: baseline; vertical-align: top"),
        );
        assert_eq!(cv.vertical_align, VerticalAlign::Top);
        let cv = cascade_doc("", "span", Some("vertical-align: bottom"));
        assert_eq!(cv.vertical_align, VerticalAlign::Bottom);
    }

    #[test]
    fn text_transform_width_keywords_cascade_as_computed_values() {
        use crate::property::TextTransform;
        let cv = cascade_doc(
            "",
            "p",
            Some("text-transform: uppercase; text-transform: full-width"),
        );
        assert_eq!(cv.text_transform, TextTransform::FullWidth);
        let cv = cascade_doc("", "p", Some("text-transform: uppercase full-width"));
        assert_eq!(cv.text_transform, TextTransform::UppercaseFullWidth);
    }

    #[test]
    fn letter_spacing_wired_through_cascade_from_inline_style() {
        // `em` (not `px`) so this also exercises phase 3 absolutization
        // (`resolve_length_or_normal`), not just the `apply_value` arm's
        // pass-through assignment.
        let cv = cascade_doc("", "p", Some("letter-spacing: 0.5em"));
        assert_eq!(cv.letter_spacing, ComputedLength(8.0));
    }

    #[test]
    fn word_spacing_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "p", Some("word-spacing: 4px"));
        assert_eq!(cv.word_spacing, ComputedLength(4.0));
    }

    #[test]
    fn word_spacing_ch_provenance_survives_inheritance() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("word-spacing: 1ch"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].word_spacing_ch_factor, Some(1.0));
        assert_eq!(r.computed[span].word_spacing_ch_factor, Some(1.0));
    }

    #[test]
    fn letter_spacing_ch_provenance_survives_inheritance() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("letter-spacing: 1ch"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing_ch_factor, Some(1.0));
        assert_eq!(r.computed[span].letter_spacing_ch_factor, Some(1.0));
    }

    #[test]
    fn letter_spacing_and_word_spacing_inherit_from_parent_element() {
        // CSS Text 3 §7.2 / §7.1: both are **inherited**.
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("letter-spacing: 2px; word-spacing: normal"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing, ComputedLength(2.0));
        assert_eq!(r.computed[p].word_spacing, ComputedLength::ZERO);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].letter_spacing,
            ComputedLength(2.0),
            "child should inherit letter-spacing from parent (CSS Text 3 §7.2 Inherited: yes)"
        );
        assert_eq!(r.computed[span].word_spacing, ComputedLength::ZERO);
    }

    #[test]
    fn letter_spacing_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("letter-spacing: 2px"));
        let span = doc.push_element(p, "span", Some("letter-spacing: -1px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing, ComputedLength(2.0));
        assert_eq!(r.computed[span].letter_spacing, ComputedLength(-1.0));
    }

    #[test]
    fn letter_spacing_inherited_em_value_does_not_re_resolve_against_child_font_size() {
        let mut doc = TestDoc::new();
        // parent: font-size 16px, letter-spacing 0.5em -> computed 8px.
        let p = doc.push_element(0, "p", Some("font-size: 16px; letter-spacing: 0.5em"));
        // child: font-size 32px, no letter-spacing declaration of its own.
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing, ComputedLength(8.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].letter_spacing,
            ComputedLength(8.0),
            "child must inherit the parent's already-computed 8px, not re-resolve \
             0.5em against its own 32px font-size (which would wrongly yield 16px)"
        );
    }

    #[test]
    fn tab_size_number_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "p", Some("tab-size: 4"));
        assert_eq!(cv.tab_size, ComputedTabSize::Number(4.0));
    }

    #[test]
    fn tab_size_length_wired_through_cascade_from_inline_style() {
        // `em` (not `px`) so this also exercises phase 3 absolutization
        // (`resolve_tab_size`), not just the `apply_value` arm's
        // pass-through assignment.
        let cv = cascade_doc("", "p", Some("font-size: 20px; tab-size: 2em"));
        assert_eq!(cv.tab_size, ComputedTabSize::Length(ComputedLength(40.0)));
    }

    #[test]
    fn tab_size_defaults_to_initial_8_when_undeclared() {
        // CSS Text Module Level 3 §4.2: "Initial: 8".
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.tab_size, ComputedTabSize::Number(8.0));
    }

    #[test]
    fn tab_size_inherits_from_parent_element() {
        // CSS Text Module Level 3 §4.2: "Inherited: yes".
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("tab-size: 6"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].tab_size, ComputedTabSize::Number(6.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].tab_size,
            ComputedTabSize::Number(6.0),
            "child should inherit tab-size from parent (CSS Text Module Level 3 §4.2 Inherited: yes)"
        );
    }

    #[test]
    fn tab_size_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("tab-size: 6"));
        let span = doc.push_element(p, "span", Some("tab-size: 2"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].tab_size, ComputedTabSize::Number(6.0));
        assert_eq!(r.computed[span].tab_size, ComputedTabSize::Number(2.0));
    }

    #[test]
    fn tab_size_inherited_em_value_does_not_re_resolve_against_child_font_size() {
        let mut doc = TestDoc::new();
        // parent: font-size 16px, tab-size 2em -> computed 32px.
        let p = doc.push_element(0, "p", Some("font-size: 16px; tab-size: 2em"));
        // child: font-size 32px, no tab-size declaration of its own.
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].tab_size,
            ComputedTabSize::Length(ComputedLength(32.0))
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].tab_size,
            ComputedTabSize::Length(ComputedLength(32.0)),
            "child must inherit the parent's already-computed 32px, not re-resolve \
             2em against its own 32px font-size (which would wrongly yield 64px)"
        );
    }

    #[test]
    fn tab_size_number_inherited_by_child_multiplies_own_font_size() {
        // Mirrors `line-height`'s unitless-number inheritance special
        // behavior shape (CSS Text Module Level 3 §4.2's `<number>`
        // alternative carries no length, so unlike the `<length>` case
        // above there is nothing to "re-resolve" — the raw number just
        // passes through unchanged regardless of the child's own font-size).
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-size: 16px; tab-size: 4"));
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].tab_size, ComputedTabSize::Number(4.0));
        assert_eq!(r.computed[span].tab_size, ComputedTabSize::Number(4.0));
    }

    #[test]
    fn text_shadow_wired_through_cascade_from_inline_style() {
        use crate::property::{CssColor, TextShadowColor};
        // `em` (not `px`) so this also exercises phase 3 absolutization
        // (`resolve_text_shadow_item`), not just the `apply_value` arm's
        // pass-through assignment — same rationale as
        // `letter_spacing_wired_through_cascade_from_inline_style`.
        let cv = cascade_doc("", "p", Some("text-shadow: 0.5em 1em red"));
        assert_eq!(
            *cv.text_shadow,
            vec![ComputedTextShadow {
                offset_x: ComputedLength(8.0),
                offset_y: ComputedLength(16.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }]
        );
    }

    #[test]
    fn text_shadow_none_is_empty_computed_list() {
        let cv = cascade_doc("", "p", Some("text-shadow: none"));
        assert!(cv.text_shadow.is_empty());
    }

    #[test]
    fn text_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade() {
        // `0e999` collapses to `NaN` internally during cssparser
        // tokenization (`raikiri-style/src/property.rs` module doc's
        // "Numeric-token NaN stabilization" section), but the acquisition
        // layer recovers the spec-correct `0.0` before
        // `parse_shadow_length_reject_nan`'s `!is_nan()` guard ever runs —
        // so `text-shadow: 0e999px 1px red` parses and cascades
        // successfully, instead of being dropped
        // (`property::tests::text_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero_but_preserves_infinity`
        // pins the parse-layer half of this). This is the end-to-end pin,
        // through the real parse -> cascade pipeline, that `0e999`
        // resolves to `0.0` all the way to `ComputedValues::text_shadow`.
        let cv = cascade_doc("", "p", Some("text-shadow: 0e999px 1px red"));
        assert_eq!(
            *cv.text_shadow,
            vec![ComputedTextShadow {
                offset_x: ComputedLength(0.0),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }]
        );
    }

    #[test]
    fn text_shadow_infinite_offset_passes_through_unclamped_through_real_cascade() {
        // `+Inf` is a *different* hazard class from `0e999`'s NaN collapse
        // above — a spec-valid `<length>` magnitude overflow (CSS Values 4
        // §5), not a `0 * Infinity` collapse — so unlike NaN it must reach
        // `ComputedValues::text_shadow` unrejected (dropping it here would
        // be the same class of regression the opacity guard's own
        // `is_finite()`-vs-`!is_nan()` history warns against).
        let cv = cascade_doc("", "p", Some("text-shadow: 1e40px 1px red"));
        assert_eq!(
            *cv.text_shadow,
            vec![ComputedTextShadow {
                offset_x: ComputedLength(f32::INFINITY),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }]
        );

        // Both signs, same reason
        // `opacity_infinite_literal_clamps_through_real_cascade_instead_of_being_dropped`
        // checks both `1e40`/`-1e40`.
        let neg = cascade_doc("", "p", Some("text-shadow: -1e40px 1px red"));
        assert_eq!(
            neg.text_shadow[0].offset_x,
            ComputedLength(f32::NEG_INFINITY)
        );
    }

    #[test]
    fn border_radius_box_shadow_and_outline_compute_through_cascade() {
        let cv = cascade_doc(
            "",
            "p",
            Some(
                "border-radius: 1em 2em 3em 4em; \
                 box-shadow: red 0.5em -1em 0.25em 0.125em, 2px 3px; \
                 outline: solid 2em red",
            ),
        );

        assert_eq!(
            cv.border_radius,
            ComputedBorderRadius {
                top_left: ComputedLengthPercentage::Px(16.0),
                top_right: ComputedLengthPercentage::Px(32.0),
                bottom_right: ComputedLengthPercentage::Px(48.0),
                bottom_left: ComputedLengthPercentage::Px(64.0),
            }
        );
        assert_eq!(
            *cv.box_shadow,
            vec![
                ComputedBoxShadowItem {
                    offset_x: ComputedLength(8.0),
                    offset_y: ComputedLength(-16.0),
                    blur_radius: ComputedLength(4.0),
                    spread_radius: ComputedLength(2.0),
                    color: TextShadowColor::Resolved(CssColor {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    }),
                    inset: false,
                },
                ComputedBoxShadowItem {
                    offset_x: ComputedLength(2.0),
                    offset_y: ComputedLength(3.0),
                    blur_radius: ComputedLength::ZERO,
                    spread_radius: ComputedLength::ZERO,
                    color: TextShadowColor::CurrentColor,
                    inset: false,
                },
            ]
        );
        assert_eq!(cv.outline.width(), ComputedLength(32.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Solid);
        assert_eq!(
            cv.outline.color,
            OutlineColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            })
        );
    }

    #[test]
    fn box_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade() {
        // Same recovery as
        // `text_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade`
        // above, for `box-shadow`
        // (`property::tests::box_shadow_zero_mantissa_huge_exponent_offset_or_spread_resolves_to_zero_but_preserves_infinity`
        // pins the parse-layer half).
        let cv = cascade_doc("", "p", Some("box-shadow: 0e999px 1px red"));
        assert_eq!(
            *cv.box_shadow,
            vec![ComputedBoxShadowItem {
                offset_x: ComputedLength(0.0),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength::ZERO,
                spread_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                inset: false,
            }]
        );
    }

    #[test]
    fn box_shadow_infinite_offset_passes_through_unclamped_through_real_cascade() {
        // `+Inf` is a *different* hazard class from `0e999`'s NaN collapse
        // above — a spec-valid `<length>` magnitude overflow (CSS Values 4
        // §5), not a `0 * Infinity` collapse — so unlike NaN it must reach
        // `ComputedValues::box_shadow` unrejected.
        let cv = cascade_doc("", "p", Some("box-shadow: 1e40px 1px 1px 1e40px red"));
        assert_eq!(
            *cv.box_shadow,
            vec![ComputedBoxShadowItem {
                offset_x: ComputedLength(f32::INFINITY),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength(1.0),
                spread_radius: ComputedLength(f32::INFINITY),
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                inset: false,
            }]
        );

        // Both signs, same reason
        // `opacity_infinite_literal_clamps_through_real_cascade_instead_of_being_dropped`
        // checks both `1e40`/`-1e40`.
        let neg = cascade_doc("", "p", Some("box-shadow: -1e40px 1px 1px -1e40px red"));
        assert_eq!(
            neg.box_shadow[0].offset_x,
            ComputedLength(f32::NEG_INFINITY)
        );
        assert_eq!(
            neg.box_shadow[0].spread_radius,
            ComputedLength(f32::NEG_INFINITY)
        );
    }

    #[test]
    fn outline_auto_cascades_as_distinct_style_from_border() {
        let cv = cascade_doc(
            "",
            "p",
            Some("border-top-width: 2px; border-top-style: solid; outline: auto 2px red"),
        );

        assert_eq!(cv.outline.width(), ComputedLength(2.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Auto);
        assert_eq!(cv.border.top.width, ComputedLength(2.0));
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
    }

    #[test]
    fn outline_none_gates_computed_width_to_zero() {
        let cv = cascade_doc("", "p", Some("outline: none 2em red"));
        assert_eq!(cv.outline.width(), ComputedLength::ZERO);
        assert_eq!(cv.outline.style(), OutlineStyle::None);
        assert_eq!(cv.outline.color, OutlineColor::Resolved(RED));
    }

    #[test]
    fn outline_color_invert_cascades_as_a_distinct_keyword() {
        let cv = cascade_doc("", "p", Some("outline-color: invert"));
        assert_eq!(cv.outline.color, OutlineColor::Invert);

        let cv = cascade_doc("", "p", Some("outline-color: currentcolor"));
        assert_eq!(cv.outline.color, OutlineColor::CurrentColor);
    }

    #[test]
    fn border_radius_box_shadow_and_outline_are_non_inherited() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(
            0,
            "p",
            Some("border-radius: 1px; box-shadow: 1px 2px red; outline: solid 3px red"),
        );
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        let initial = ComputedValues::initial();

        assert_ne!(result.computed[parent].border_radius, initial.border_radius);
        assert!(!result.computed[parent].box_shadow.is_empty());
        assert_ne!(result.computed[parent].outline, initial.outline);
        assert_eq!(result.computed[child].border_radius, initial.border_radius);
        assert_eq!(result.computed[child].box_shadow, initial.box_shadow);
        assert_eq!(result.computed[child].outline, initial.outline);
    }

    #[test]
    fn text_shadow_inherits_from_parent_element() {
        // CSS Text Decoration Module Level 3 §4: **inherited**.
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-shadow: 1px 1px black"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_shadow.len(), 1);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_shadow, r.computed[p].text_shadow,
            "child should inherit text-shadow from parent (CSS Text Decoration \
             Module Level 3 §4 Inherited: yes)"
        );
    }

    #[test]
    fn text_shadow_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-shadow: 1px 1px black"));
        let span = doc.push_element(p, "span", Some("text-shadow: none"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_shadow.len(), 1);
        assert!(r.computed[span].text_shadow.is_empty());
    }

    #[test]
    fn text_shadow_inherited_em_value_does_not_re_resolve_against_child_font_size() {
        let mut doc = TestDoc::new();
        // parent: font-size 16px, text-shadow 0.5em -> computed 8px.
        let p = doc.push_element(0, "p", Some("font-size: 16px; text-shadow: 0.5em 0.5em"));
        // child: font-size 32px, no text-shadow declaration of its own.
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_shadow[0].offset_x, ComputedLength(8.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_shadow[0].offset_x,
            ComputedLength(8.0),
            "child must inherit the parent's already-computed 8px, not re-resolve \
             0.5em against its own 32px font-size (which would wrongly yield 16px)"
        );
    }

    #[test]
    fn white_space_wired_through_cascade_from_inline_style() {
        use crate::property::WhiteSpace;
        let cv = cascade_doc("", "p", Some("white-space: pre"));
        assert_eq!(cv.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn white_space_inherits_from_parent_element() {
        // CSS Text 3 §3: white-space は **inherited**.
        use crate::property::WhiteSpace;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("white-space: pre"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].white_space, WhiteSpace::Pre);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].white_space,
            WhiteSpace::Pre,
            "child should inherit white-space from parent (CSS Text 3 §3 Inherited: yes)"
        );
    }

    #[test]
    fn white_space_child_own_value_wins_over_inherited() {
        use crate::property::WhiteSpace;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("white-space: pre"));
        let span = doc.push_element(p, "span", Some("white-space: nowrap"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].white_space, WhiteSpace::Pre);
        assert_eq!(r.computed[span].white_space, WhiteSpace::Nowrap);
    }

    #[test]
    fn hyphens_wired_through_cascade_from_inline_style() {
        use crate::property::Hyphens;
        let cv = cascade_doc("", "p", Some("hyphens: auto"));
        assert_eq!(cv.hyphens, Hyphens::Auto);
    }

    #[test]
    fn hyphens_inherits_from_parent_element() {
        // CSS Text 3 §5.3: hyphens は **inherited**.
        use crate::property::Hyphens;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("hyphens: auto"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].hyphens, Hyphens::Auto);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].hyphens,
            Hyphens::Auto,
            "child should inherit hyphens from parent (CSS Text 3 §5.3 Inherited: yes)"
        );
    }

    #[test]
    fn hyphens_child_own_value_wins_over_inherited() {
        use crate::property::Hyphens;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("hyphens: auto"));
        let span = doc.push_element(p, "span", Some("hyphens: none"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].hyphens, Hyphens::Auto);
        assert_eq!(r.computed[span].hyphens, Hyphens::None);
    }

    #[test]
    fn writing_mode_wired_through_cascade_from_inline_style() {
        use crate::property::WritingMode;
        let cv = cascade_doc("", "p", Some("writing-mode: vertical-rl"));
        // `apply_value`'s `WritingMode` arm assigns the raw specified
        // keyword; the `HorizontalTb` collapse for non-horizontal keywords
        // happens later in `absolutize_with` (see `apply_value`'s
        // `PropertyValue::WritingMode` arm doc comment), so the computed
        // value here is always `HorizontalTb` even for `vertical-rl`.
        // Future work: vertical writing 実装時に
        // `HorizontalTb` 期待値を `VerticalRl` へ戻すこと。
        assert_eq!(cv.writing_mode, WritingMode::HorizontalTb);
    }

    #[test]
    fn ruby_position_wired_through_cascade_and_inheritance() {
        use crate::property::RubyPosition;
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "p", Some("ruby-position: under"));
        let child = doc.push_element(parent, "ruby", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[parent].ruby_position, RubyPosition::Under);
        assert_eq!(result.computed[child].ruby_position, RubyPosition::Under);
    }

    #[test]
    fn authored_writing_mode_is_retained_before_computed_normalization() {
        use crate::property::WritingMode;
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("writing-mode: vertical-rl"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.authored_writing_modes[root],
            Some(WritingMode::VerticalRl)
        );
        assert_eq!(
            result.computed[root].writing_mode,
            WritingMode::HorizontalTb
        );
    }

    #[test]
    fn page_margin_inherit_uses_the_html_root_element() {
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", Some("margin: 0.5in"));
        let style = doc.push_element(html, "style", None);
        doc.push_text(style, "@page { margin: 13px; margin: inherit }");
        let tree = build_rule_tree(&doc);
        let result = cascade_with_media_context_for_page(
            &doc,
            &tree,
            &MediaContext::default(),
            &crate::page::PageContextQuery::default(),
        )
        .expect("page cascade Ok");
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                48.0
            ))))
        );
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginRight),
            Some(&PropertyValue::MarginRight(LengthOrAuto::Length(
                Length::Px(48.0)
            )))
        );
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginBottom),
            Some(&PropertyValue::MarginBottom(LengthOrAuto::Length(
                Length::Px(48.0)
            )))
        );
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginLeft),
            Some(&PropertyValue::MarginLeft(LengthOrAuto::Length(
                Length::Px(48.0)
            )))
        );
    }

    #[test]
    fn page_margin_inherit_preserves_computed_margin_value_shapes() {
        let mut root = ComputedValues::initial();
        root.margin = Sides {
            top: ComputedLengthPercentageOrAuto::Auto,
            right: ComputedLengthPercentageOrAuto::Px(12.0),
            bottom: ComputedLengthPercentageOrAuto::Percent(25.0),
            left: ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
                percent: 10.0,
                px: 2.0,
            }),
        };
        let ctx = ResolveContext::new(root.font_size);
        let cases = [
            (
                PropertyValue::MarginTopInherit,
                PropertyValue::MarginTop(LengthOrAuto::Auto),
            ),
            (
                PropertyValue::MarginRightInherit,
                PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(12.0))),
            ),
            (
                PropertyValue::MarginBottomInherit,
                PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Percent(25.0))),
            ),
            (
                PropertyValue::MarginLeftInherit,
                PropertyValue::MarginLeft(LengthOrAuto::Calc(CalcLengthPercentage {
                    percent: 10.0,
                    px: 2.0,
                })),
            ),
        ];
        for (marker, expected) in cases {
            assert_eq!(
                resolve_against_inherited(marker, &root, &ctx).into_property_value(),
                expected
            );
        }
        assert_eq!(
            resolve_against_inherited(PropertyValue::MarginInherit, &root, &ctx)
                .into_property_value(),
            PropertyValue::Margin(root.margin.map(|value| match value {
                ComputedLengthPercentageOrAuto::Px(px) => LengthOrAuto::Length(Length::Px(px)),
                ComputedLengthPercentageOrAuto::Percent(percent) => {
                    LengthOrAuto::Length(Length::Percent(percent))
                }
                ComputedLengthPercentageOrAuto::Calc(calc) => LengthOrAuto::Calc(calc),
                ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            }))
        );
    }

    #[test]
    fn background_repeat_attachment_clip_origin_wired_through_cascade_from_inline_style() {
        use crate::property::{
            BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, VisualBox,
        };
        let cv = cascade_doc(
            "",
            "div",
            Some(
                "background-repeat: repeat-x; background-attachment: fixed; \
                 background-clip: content-box; background-origin: border-box",
            ),
        );
        assert_eq!(
            cv.background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::NoRepeat,
            }
        );
        assert_eq!(cv.background_attachment, BackgroundAttachment::Fixed);
        assert_eq!(cv.background_clip, VisualBox::ContentBox);
        assert_eq!(cv.background_origin, VisualBox::BorderBox);
    }

    #[test]
    fn background_repeat_attachment_clip_origin_are_non_inherited() {
        use crate::property::{
            BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, VisualBox,
        };
        let mut doc = TestDoc::new();
        let p = doc.push_element(
            0,
            "p",
            Some(
                "background-repeat: round; background-attachment: local; \
                 background-clip: content-box; background-origin: content-box",
            ),
        );
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Round,
                y: BackgroundRepeatKeyword::Round,
            }
        );
        // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8 "Inherited: no" — the
        // child without its own winner resets to each property's spec
        // initial, not the parent's value.
        assert_eq!(
            r.computed[span].background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            }
        );
        assert_eq!(
            r.computed[p].background_attachment,
            BackgroundAttachment::Local
        );
        assert_eq!(
            r.computed[span].background_attachment,
            BackgroundAttachment::Scroll
        );
        assert_eq!(r.computed[p].background_clip, VisualBox::ContentBox);
        assert_eq!(r.computed[span].background_clip, VisualBox::BorderBox);
        assert_eq!(r.computed[p].background_origin, VisualBox::ContentBox);
        // `background-origin`'s initial (`padding-box`) differs from
        // `background-clip`'s (`border-box`) — check both distinctly.
        assert_eq!(r.computed[span].background_origin, VisualBox::PaddingBox);
    }

    #[test]
    fn background_size_wired_through_cascade_and_absolutizes_em() {
        use crate::resolve::{ComputedBackgroundSize, ComputedLengthPercentageOrAuto};
        // `2em` at the default 16px font-size absolutizes to 32px; the 2nd
        // axis is omitted so it fills with `auto` (not a duplicate of the
        // 1st, per `BackgroundSize` doc's fill-rule note).
        let cv = cascade_doc("", "div", Some("background-size: 2em"));
        assert_eq!(
            cv.background_size,
            ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Px(32.0),
                height: ComputedLengthPercentageOrAuto::Auto,
            }
        );
    }

    #[test]
    fn background_size_is_non_inherited() {
        use crate::resolve::ComputedBackgroundSize;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-size: cover"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].background_size, ComputedBackgroundSize::Cover);
        assert_eq!(
            r.computed[span].background_size,
            ComputedValues::initial().background_size
        );
    }

    #[test]
    fn background_position_wired_through_cascade_and_absolutizes_edge_offset() {
        use crate::resolve::{
            ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
        };
        // `bottom 1em right` — `1em` absolutizes to 16px at the default
        // font-size, `right`'s omitted offset defaults to 0 and normalizes
        // to `Start(100%)` (`CssPositionOffset` doc's normalization note).
        let cv = cascade_doc("", "div", Some("background-position: bottom 1em right"));
        assert_eq!(
            cv.background_position,
            ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    100.0
                )),
                vertical: ComputedCssPositionOffset::End(ComputedLengthPercentage::Px(16.0)),
            }
        );
    }

    #[test]
    fn background_position_is_non_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-position: right bottom"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_ne!(
            r.computed[p].background_position,
            ComputedValues::initial().background_position
        );
        assert_eq!(
            r.computed[span].background_position,
            ComputedValues::initial().background_position
        );
    }

    #[test]
    fn object_fit_wired_through_cascade_from_inline_style() {
        use crate::property::ObjectFit;
        let cv = cascade_doc("", "img", Some("object-fit: contain"));
        assert_eq!(cv.object_fit, ObjectFit::Contain);
    }

    #[test]
    fn object_fit_defaults_to_fill_without_declaration() {
        use crate::property::ObjectFit;
        let cv = cascade_doc("", "img", None);
        assert_eq!(cv.object_fit, ObjectFit::Fill);
    }

    #[test]
    fn object_fit_is_non_inherited() {
        use crate::property::ObjectFit;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("object-fit: cover"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].object_fit, ObjectFit::Cover);
        // CSS Images Module Level 3 §5.1 "Inherited: no" — the child without
        // its own winner resets to the spec initial, not the parent's value
        // (`background_position_is_non_inherited` sibling shape above).
        assert_eq!(r.computed[span].object_fit, ObjectFit::Fill);
    }

    #[test]
    fn object_position_wired_through_cascade_and_absolutizes_em() {
        use crate::resolve::{
            ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
        };
        // `2em` at the default 16px font-size absolutizes to 32px.
        let cv = cascade_doc("", "img", Some("object-position: 2em 10%"));
        assert_eq!(
            cv.object_position,
            ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(32.0)),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(10.0)),
            }
        );
    }

    #[test]
    fn object_position_defaults_to_50_percent_50_percent_without_declaration() {
        // CSS Images Module Level 3 §5.2 "Initial: 50% 50%" — distinct from
        // `background-position`'s `0% 0%` initial
        // (`background_position_is_non_inherited` sibling above resets to
        // `0% 0%`; this property resets to `50% 50%` instead).
        use crate::resolve::{
            ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
        };
        let cv = cascade_doc("", "img", None);
        assert_eq!(
            cv.object_position,
            ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    50.0
                )),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(50.0)),
            }
        );
    }

    #[test]
    fn object_position_is_non_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("object-position: right bottom"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_ne!(
            r.computed[p].object_position,
            ComputedValues::initial().object_position
        );
        // CSS Images Module Level 3 §5.2 "Inherited: no" — the child without
        // its own winner resets to the spec initial (`50% 50%`), not the
        // parent's value.
        assert_eq!(
            r.computed[span].object_position,
            ComputedValues::initial().object_position
        );
    }

    #[test]
    fn object_position_3_value_edge_offset_form_dropped_through_real_cascade() {
        // `right 10px center` is `<bg-position>`'s (CSS Backgrounds 3 §2.6)
        // 3-value extension, not valid for `object-position`'s plain
        // `<position>` (CSS Values 4 §8.3) —
        // `object_position_rejects_bg_position_only_3_value_edge_offset_forms`
        // (property.rs) pins this at the `parse_value` level; this is the
        // end-to-end sibling through the real parse -> cascade pipeline
        // (`rule::DeclParser`'s `expect_exhausted` drops the whole
        // declaration once `parse_position_strict`'s fallback alternative
        // leaves `center` as leftover, same mechanism as the
        // `background_shorthand_*_dropped_through_real_cascade` tests).
        let cv = cascade_doc("", "img", Some("object-position: right 10px center"));
        assert_eq!(
            cv.object_position,
            ComputedValues::initial().object_position
        );
    }

    #[test]
    fn opacity_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "div", Some("opacity: 0.5"));
        assert_eq!(cv.opacity, 0.5);
    }

    #[test]
    fn opacity_defaults_to_1_without_declaration() {
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.opacity, 1.0);
    }

    #[test]
    fn opacity_is_non_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("opacity: 0.3"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].opacity, 0.3);
        // CSS Color 4 §3.3 "Inherited: no" — the child without its own
        // winner resets to the spec initial (`1`), not the parent's value
        // (`object_fit_is_non_inherited` sibling shape above).
        assert_eq!(r.computed[span].opacity, 1.0);
    }

    #[test]
    fn opacity_out_of_range_clamps_to_0_1_through_real_cascade() {
        // CSS Color 4 §3.3: "clamped to the range `[0, 1]` in computed
        // values" — the clamp is a phase-3 transform
        // (`SpecifiedValues::absolutize_with`), pinned end-to-end here
        // through the real parse -> cascade pipeline (unit-level check at
        // `SpecifiedValues::finalize` itself is
        // `opacity_out_of_range_specified_clamps_at_finalize` in
        // `specified.rs`).
        let over = cascade_doc("", "div", Some("opacity: 2"));
        assert_eq!(over.opacity, 1.0);
        let under = cascade_doc("", "div", Some("opacity: -3"));
        assert_eq!(under.opacity, 0.0);
    }

    #[test]
    fn opacity_percentage_wired_through_cascade_and_clamps() {
        // `150%` exercises the percentage branch of `<opacity-value>` (CSS
        // Color 4 §3.3) through the same clamp, a distinct code path from
        // the bare-`<number>` case above.
        let cv = cascade_doc("", "div", Some("opacity: 150%"));
        assert_eq!(cv.opacity, 1.0);
        let half = cascade_doc("", "div", Some("opacity: 50%"));
        assert_eq!(half.opacity, 0.5);
    }

    #[test]
    fn opacity_zero_mantissa_huge_exponent_resolves_through_real_cascade() {
        // `0e999` collapses to `NaN` internally during cssparser
        // tokenization (`raikiri-style/src/property.rs` module doc's
        // "Numeric-token NaN stabilization" section), but the acquisition
        // layer recovers the spec-correct `0.0` before `parse_opacity_value`'s
        // `!is_nan()` guard ever runs — so `opacity: 0e999` parses and
        // cascades successfully to `0.0`, not dropped back to the initial
        // `1.0`
        // (`property::tests::opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity`
        // pins the parse-layer half of this). This is the end-to-end pin,
        // through the real parse -> cascade pipeline, that `0e999` reaches
        // `ComputedValues::opacity` as `0.0`.
        let cv = cascade_doc("", "div", Some("opacity: 0e999"));
        assert_eq!(cv.opacity, 0.0);
        assert!(!cv.opacity.is_nan());
    }

    #[test]
    fn opacity_infinite_literal_clamps_through_real_cascade_instead_of_being_dropped() {
        // `1e40`/`-1e40` overflow to `+Inf`/`-Inf` during tokenization — a
        // *different* hazard class from `0e999`'s NaN collapse above
        // (`parse_opacity_value` doc's "`!is_nan()` guard" section). Unlike
        // NaN, these are spec-valid `<number>` values that must reach the
        // phase-3 clamp and become `1.0`/`0.0` — not be dropped and fall
        // back to the initial `1.0` (which would silently turn a
        // fully-transparent `-1e40` into fully opaque, a regression an
        // earlier iteration of the parse-time guard introduced by using
        // `is_finite()` instead of `!is_nan()`).
        let over = cascade_doc("", "div", Some("opacity: 1e40"));
        assert_eq!(over.opacity, 1.0);
        let under = cascade_doc("", "div", Some("opacity: -1e40"));
        assert_eq!(under.opacity, 0.0);
    }

    #[test]
    fn isolation_wired_through_cascade_from_inline_style() {
        use crate::property::Isolation;
        let cv = cascade_doc("", "div", Some("isolation: isolate"));
        assert_eq!(cv.isolation, Isolation::Isolate);
    }

    #[test]
    fn isolation_defaults_to_auto_without_declaration() {
        use crate::property::Isolation;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.isolation, Isolation::Auto);
    }

    #[test]
    fn isolation_is_non_inherited() {
        use crate::property::Isolation;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("isolation: isolate"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].isolation, Isolation::Isolate);
        assert_eq!(r.computed[span].isolation, Isolation::Auto);
    }

    #[test]
    fn mix_blend_mode_wired_through_cascade_from_inline_style() {
        use crate::property::MixBlendMode;
        let cv = cascade_doc("", "div", Some("mix-blend-mode: multiply"));
        assert_eq!(cv.mix_blend_mode, MixBlendMode::Multiply);
    }

    #[test]
    fn mix_blend_mode_defaults_to_normal_without_declaration() {
        use crate::property::MixBlendMode;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.mix_blend_mode, MixBlendMode::Normal);
    }

    #[test]
    fn mix_blend_mode_is_non_inherited() {
        use crate::property::MixBlendMode;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("mix-blend-mode: screen"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].mix_blend_mode, MixBlendMode::Screen);
        assert_eq!(r.computed[span].mix_blend_mode, MixBlendMode::Normal);
    }

    #[test]
    fn mask_image_wired_through_cascade_from_inline_style() {
        use crate::property::MaskImage;
        let cv = cascade_doc("", "div", Some("mask-image: url(mask.svg)"));
        assert_eq!(cv.mask_image, MaskImage::Url("mask.svg".to_string()));
    }

    #[test]
    fn mask_image_defaults_to_none_without_declaration() {
        use crate::property::MaskImage;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.mask_image, MaskImage::None);
    }

    #[test]
    fn mask_image_is_non_inherited() {
        use crate::property::MaskImage;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("mask-image: url(mask.svg)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].mask_image,
            MaskImage::Url("mask.svg".to_string())
        );
        assert_eq!(r.computed[span].mask_image, MaskImage::None);
    }

    #[test]
    fn clip_path_wired_through_cascade_from_inline_style() {
        use crate::property::{ClipPath, GeometryBox};
        let cv = cascade_doc("", "div", Some("clip-path: padding-box"));
        assert_eq!(cv.clip_path, ClipPath::GeometryBox(GeometryBox::PaddingBox));
    }

    #[test]
    fn clip_path_defaults_to_none_without_declaration() {
        use crate::property::ClipPath;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.clip_path, ClipPath::None);
    }

    #[test]
    fn clip_path_is_non_inherited() {
        use crate::property::{ClipPath, GeometryBox};
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("clip-path: border-box"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].clip_path,
            ClipPath::GeometryBox(GeometryBox::BorderBox)
        );
        assert_eq!(r.computed[span].clip_path, ClipPath::None);
    }

    #[test]
    fn transform_wired_through_cascade_from_inline_style() {
        use crate::property::Angle;
        use crate::resolve::ComputedTransformFunction;
        let cv = cascade_doc("", "div", Some("transform: rotate(45deg)"));
        assert_eq!(
            *cv.transform,
            vec![ComputedTransformFunction::Rotate(Angle(45.0))]
        );
    }

    #[test]
    fn transform_defaults_to_none_without_declaration() {
        let cv = cascade_doc("", "div", None);
        assert!(cv.transform.is_empty());
    }

    #[test]
    fn transform_is_non_inherited() {
        use crate::property::Angle;
        use crate::resolve::ComputedTransformFunction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("transform: rotate(45deg)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            *r.computed[p].transform,
            vec![ComputedTransformFunction::Rotate(Angle(45.0))]
        );
        assert!(r.computed[span].transform.is_empty());
    }

    #[test]
    fn filter_wired_through_cascade_from_inline_style() {
        use crate::property::FilterFunction;
        let cv = cascade_doc("", "div", Some("filter: blur(2px)"));
        assert_eq!(
            *cv.filter,
            vec![FilterFunction::Blur(crate::property::Length::Px(2.0))]
        );
    }

    #[test]
    fn filter_defaults_to_none_without_declaration() {
        let cv = cascade_doc("", "div", None);
        assert!(cv.filter.is_empty());
    }

    #[test]
    fn filter_is_non_inherited() {
        use crate::property::FilterFunction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("filter: blur(2px)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            *r.computed[p].filter,
            vec![FilterFunction::Blur(crate::property::Length::Px(2.0))]
        );
        assert!(r.computed[span].filter.is_empty());
    }

    #[test]
    fn filter_drop_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade() {
        use crate::property::{FilterFunction, TextShadowItem};
        // `0e999` collapses to `NaN` internally during cssparser
        // tokenization, but the acquisition layer recovers the
        // spec-correct `0.0` before `parse_shadow_length_reject_nan`'s
        // `!is_nan()` guard ever runs (reused verbatim by
        // `parse_drop_shadow_args`) — so `filter: drop-shadow(0e999px 2px)`
        // parses and cascades successfully, instead of being dropped
        // (`property::tests::filter_drop_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero`
        // pins the parse-layer half of this). This is the end-to-end pin,
        // through the real parse -> cascade pipeline, that `0e999`
        // resolves to `0.0` all the way to `ComputedValues::filter`.
        let cv = cascade_doc("", "div", Some("filter: drop-shadow(0e999px 2px)"));
        assert_eq!(
            *cv.filter,
            vec![FilterFunction::DropShadow(TextShadowItem {
                offset_x: Length::Px(0.0),
                offset_y: Length::Px(2.0),
                blur_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
            })]
        );
    }

    #[test]
    fn filter_drop_shadow_infinite_offset_passes_through_unclamped_through_real_cascade() {
        use crate::property::{FilterFunction, TextShadowItem};
        // `+Inf` is a *different* hazard class from `0e999`'s NaN collapse
        // above — a spec-valid `<length>` magnitude overflow (CSS Values 4
        // §5), not a `0 * Infinity` collapse — so unlike NaN it must reach
        // `ComputedValues::filter` unrejected. `filter` is not
        // independently absolutized at phase 3 (unlike `text-shadow`/
        // `box-shadow`), so the payload stays the specified `Length`/
        // `TextShadowItem` shape all the way through.
        let cv = cascade_doc("", "div", Some("filter: drop-shadow(1e40px 2px)"));
        assert_eq!(
            *cv.filter,
            vec![FilterFunction::DropShadow(TextShadowItem {
                offset_x: Length::Px(f32::INFINITY),
                offset_y: Length::Px(2.0),
                blur_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
            })]
        );
    }

    #[test]
    fn background_image_wired_through_cascade_from_inline_style() {
        use crate::property::BackgroundImage;
        let cv = cascade_doc("", "div", Some("background-image: url(marble.svg)"));
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("marble.svg".to_string())
        );
    }

    #[test]
    fn background_image_defaults_to_none_without_declaration() {
        // No `background-image` declaration at all — spec initial (`none`).
        // Complements `background_image_explicit_none_overrides_an_earlier_url`
        // below, which exercises the *parsed* `none` keyword end-to-end
        // rather than just the no-winner default.
        use crate::property::BackgroundImage;
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.background_image, BackgroundImage::None);
    }

    #[test]
    fn background_image_explicit_none_overrides_an_earlier_url() {
        // CSS Cascading Level 4 §6.1: the later `none` declaration wins.
        use crate::property::BackgroundImage;
        let cv = cascade_doc(
            "",
            "div",
            Some("background-image: url(a.png); background-image: none"),
        );
        assert_eq!(cv.background_image, BackgroundImage::None);
    }

    #[test]
    fn background_image_is_non_inherited() {
        use crate::property::BackgroundImage;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-image: url(marble.svg)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_image,
            BackgroundImage::Url("marble.svg".to_string())
        );
        // CSS Backgrounds and Borders 3 §2.3 "Inherited: no" — the child
        // without its own winner resets to the spec initial (`none`), not
        // the parent's value.
        assert_eq!(r.computed[span].background_image, BackgroundImage::None);
    }

    #[test]
    fn background_image_wired_through_cascade_from_inline_style_with_gradient() {
        use crate::property::{BackgroundImage, Gradient};
        // `<gradient>` (`linear-gradient()` etc.) wires through the cascade
        // like any other `BackgroundImage` payload — no dedicated cascade.rs
        // match arm exists for it (`apply_value`'s `BackgroundImage(v) =>
        // target.background_image = v` arm takes any payload via wildcard),
        // so this pins the integration rather than exercising new cascade logic.
        let cv = cascade_doc(
            "",
            "div",
            Some("background-image: linear-gradient(red, blue)"),
        );
        assert!(matches!(
            cv.background_image,
            BackgroundImage::Gradient(Gradient::Linear(_))
        ));
    }

    #[test]
    fn background_image_invalid_gradient_does_not_overwrite_an_earlier_url() {
        // Mirrors `background_image_explicit_none_overrides_an_earlier_url`'s
        // same-block later-declaration-wins setup, but with a *syntactically
        // invalid* later declaration (a single-stop `linear-gradient()` —
        // the CSS Images 3 baseline grammar this crate implements requires
        // 2+ stops, `BackgroundImage` doc's scope-carving section) instead
        // of a valid one: the whole invalid declaration must drop, leaving
        // the earlier `url(...)` winner untouched, not coerced to the
        // property's initial value.
        use crate::property::BackgroundImage;
        let cv = cascade_doc(
            "",
            "div",
            Some("background-image: url(a.png); background-image: linear-gradient(red)"),
        );
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("a.png".to_string())
        );
    }

    #[test]
    fn background_image_radial_gradient_position_before_shape_does_not_overwrite_an_earlier_url() {
        // Same shape as `background_image_invalid_gradient_does_not_overwrite_an_earlier_url`,
        // but with a different flavor of syntactically-invalid gradient:
        // `at center circle` violates CSS Images 4 §3.2.1's `[ [
        // <radial-shape> || <radial-size> ]? [ at <position> ]? ]`
        // sequencing (`at <position>` may only follow the shape/size group,
        // never precede it) — without this test, a regression that widens
        // `parse_radial_gradient_body` back to a flat any-order loop over
        // shape/size/position (rather than treating shape/size/position as
        // one ordered group) would *accept* this declaration and overwrite
        // the earlier `url(...)` winner with a spec-invalid gradient,
        // silently corrupting the cascade result instead of failing loudly.
        use crate::property::BackgroundImage;
        let cv = cascade_doc(
            "",
            "div",
            Some(
                "background-image: url(a.png); background-image: radial-gradient(at center circle, red, blue)",
            ),
        );
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("a.png".to_string())
        );
    }

    #[test]
    fn orphans_widows_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "p", Some("orphans: 4; widows: 3"));
        assert_eq!(cv.orphans, 4);
        assert_eq!(cv.widows, 3);
    }

    #[test]
    fn orphans_widows_inherit_from_parent_element() {
        // CSS Fragmentation Module Level 3 §3.3: orphans / widows は
        // **inherited**.
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("orphans: 4; widows: 3"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].orphans, 4);
        assert_eq!(r.computed[p].widows, 3);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].orphans, 4,
            "child should inherit orphans from parent (CSS Fragmentation \
             Module Level 3 §3.3 Inherited: yes)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].widows, 3,
            "child should inherit widows from parent (CSS Fragmentation \
             Module Level 3 §3.3 Inherited: yes)"
        );
    }

    #[test]
    fn orphans_widows_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("orphans: 4; widows: 3"));
        let span = doc.push_element(p, "span", Some("orphans: 6; widows: 5"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].orphans, 4);
        assert_eq!(r.computed[p].widows, 3);
        assert_eq!(r.computed[span].orphans, 6);
        assert_eq!(r.computed[span].widows, 5);
    }

    #[test]
    fn orphans_widows_default_to_initial_value_2_without_declaration() {
        // CSS Fragmentation Module Level 3 §3.3: Initial は共に `2`。
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.orphans, 2);
        assert_eq!(cv.widows, 2);
    }

    #[test]
    fn orphans_widows_reject_zero_and_negative_leaving_initial_value() {
        // "Negative values and zero are invalid and must cause the
        // declaration to be ignored" — the whole declaration drops, so the
        // property stays at its initial value `2` rather than being clamped.
        let cv = cascade_doc("", "p", Some("orphans: 0; widows: -1"));
        assert_eq!(cv.orphans, 2);
        assert_eq!(cv.widows, 2);
    }

    #[test]
    fn orphans_widows_invalid_declaration_is_dropped_independently_of_sibling() {
        // The spec says "the declaration" (singular) is ignored — this pins
        // that an invalid `orphans` doesn't also take down a syntactically
        // valid, separately-declared `widows` in the same block (per-
        // declaration drop, not per-block).
        let cv = cascade_doc("", "p", Some("orphans: 0; widows: 3"));
        assert_eq!(cv.orphans, 2);
        assert_eq!(cv.widows, 3);
    }

    #[test]
    fn text_align_match_parent_resolves_start_against_ltr_parent_to_left() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        // Parent: `text-align: start` explicit, `direction` defaults to `ltr`.
        let p = doc.push_element(0, "p", Some("text-align: start"));
        let span = doc.push_element(p, "span", Some("text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Left,
            "start + ltr → left (CSS Text 3 §6.1 match-parent table)"
        );
    }

    #[test]
    fn text_align_match_parent_uses_parent_direction_not_own_declared_direction() {
        use crate::property::{Direction, TextAlign};
        let mut doc = TestDoc::new();
        // Parent: direction defaults to ltr, text-align defaults to start.
        let p = doc.push_element(0, "p", None);
        // Child: declares its own (conflicting) direction *and* match-parent.
        let span = doc.push_element(p, "span", Some("direction: rtl; text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Ltr);
        assert_eq!(r.computed[p].text_align, TextAlign::Start);
        // Own `direction: rtl` still applies to the child normally — it's a
        // separate property, unaffected by the match-parent resolution.
        assert_eq!(r.computed[span].direction, Direction::Rtl);
        // But `text-align: match-parent` must resolve against the *parent's*
        // ltr (→ left), not the child's own rtl (which would give right).
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Left,
            "match-parent must use the parent's direction, not the node's own direction winner (CSS Text 3 §6.1 verbatim: \"the parent's direction value\")"
        );
    }

    #[test]
    fn text_align_match_parent_resolves_end_against_rtl_parent_to_left() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl; text-align: end"));
        let span = doc.push_element(p, "span", Some("text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Left,
            "end + rtl → left (CSS Text 3 §6.1 match-parent table)"
        );
    }

    #[test]
    fn text_align_match_parent_copies_center_parent_verbatim() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align: center"));
        let span = doc.push_element(p, "span", Some("text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[span].text_align, TextAlign::Center);
    }

    #[test]
    fn text_align_match_parent_on_root_element_resolves_to_start() {
        use crate::property::TextAlign;
        let cv = cascade_doc("", "html", Some("direction: rtl; text-align: match-parent"));
        assert_eq!(cv.text_align, TextAlign::Start);
    }

    #[test]
    fn text_align_match_parent_resolves_against_already_resolved_parent() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "a", Some("text-align: start")); // ltr default → resolves nowhere (not match-parent itself)
        let b = doc.push_element(a, "b", Some("text-align: match-parent")); // start+ltr → left
        let c = doc.push_element(b, "c", Some("text-align: match-parent")); // inherits `left` verbatim
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].text_align, TextAlign::Start);
        assert_eq!(r.computed[b].text_align, TextAlign::Left);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[c].text_align,
            TextAlign::Left,
            "grandchild's match-parent must copy the child's *resolved* Left, not re-interpret MatchParent"
        );
    }

    #[test]
    fn box_sizing_wired_through_cascade_from_inline_style() {
        // <p style="box-sizing: border-box"> → ComputedValues.box_sizing に
        // BoxSizing::BorderBox が届く。parser → PropertyValue::BoxSizing →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // sibling (background-color / line-height / counter-* / content /
        // string-set / position / text-align) の wire-through pattern を踏襲
        // (原則 1 前例主義)。
        use crate::property::BoxSizing;
        let cv = cascade_doc("", "p", Some("box-sizing: border-box"));
        assert_eq!(cv.box_sizing, BoxSizing::BorderBox);
    }

    #[test]
    fn vertical_align_new_keywords_wired_through_cascade_from_inline_style() {
        // parser → PropertyValue::VerticalAlign → apply_value →
        // SpecifiedValues::finalize → ComputedValues の end-to-end 疎通 —
        // `middle`/`text-top`/`text-bottom` (`box_sizing_wired_through_cascade_from_inline_style`
        // と同じ pattern)。
        use crate::property::VerticalAlign;
        assert_eq!(
            cascade_doc("", "span", Some("vertical-align: middle")).vertical_align,
            VerticalAlign::Middle
        );
        assert_eq!(
            cascade_doc("", "span", Some("vertical-align: text-top")).vertical_align,
            VerticalAlign::TextTop
        );
        assert_eq!(
            cascade_doc("", "span", Some("vertical-align: text-bottom")).vertical_align,
            VerticalAlign::TextBottom
        );
    }

    #[test]
    fn vertical_align_length_absolutizes_against_own_font_size_through_cascade() {
        // `font-size: 20px; vertical-align: 2em` on the same element →
        // phase 3 (`resolve_vertical_align`, called from
        // `SpecifiedValues::absolutize_with`) absolutizes against this
        // element's own (already phase-2-resolved) `font-size`, not the
        // inherited parent's — 2 * 20 = 40px.
        use crate::property::{Length, VerticalAlign};
        let cv = cascade_doc("", "span", Some("font-size: 20px; vertical-align: 2em"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.vertical_align, VerticalAlign::Length(Length::Px(40.0)));
    }

    #[test]
    fn z_index_wired_through_cascade_from_inline_style() {
        // <p style="z-index: 3"> → ComputedValues.z_index に
        // ZIndexValue::Integer(3) が届く。parser → PropertyValue::ZIndex →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。sibling
        // (box-sizing / font-style) の wire-through pattern を踏襲。
        use crate::property::ZIndexValue;
        let cv = cascade_doc("", "p", Some("z-index: 3"));
        assert_eq!(cv.z_index, ZIndexValue::Integer(3));
    }

    #[test]
    fn order_wired_through_cascade_from_inline_style() {
        // <p style="order: 2"> → ComputedValues.order に 2 が届く。parser →
        // PropertyValue::Order → apply_value → ComputedValues の end-to-end
        // 疎通 smoke (`z_index_wired_through_cascade_from_inline_style` と
        // 同 pattern)。
        let cv = cascade_doc("", "p", Some("order: 2"));
        assert_eq!(cv.order, 2);
    }

    #[test]
    fn order_non_inherited_child_starts_from_initial() {
        // CSS Flexible Box Layout Module Level 1 §4.2 propdef:
        // "Inherited: no" (`z_index_non_inherited_child_starts_from_initial`
        // と同じ pattern)。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("order: 5"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].order, 5);
        assert_eq!(
            r.computed[span].order, 0,
            "order must not inherit from parent (CSS Flexbox 1 §4.2 Inherited: no)"
        );
    }

    #[test]
    fn flex_flow_shorthand_wired_through_cascade_from_inline_style() {
        // <p style="flex-flow: column wrap"> → ComputedValues.flex_direction /
        // flex_wrap に Column / Wrap が届く (parse → expand → apply_value の
        // end-to-end 疎通 smoke)。
        use crate::property::{FlexDirectionValue, FlexWrapValue};
        let cv = cascade_doc("", "p", Some("flex-flow: column wrap"));
        assert_eq!(cv.flex_direction, FlexDirectionValue::Column);
        assert_eq!(cv.flex_wrap, FlexWrapValue::Wrap);
    }

    #[test]
    fn z_index_non_inherited_child_starts_from_initial() {
        // CSS2 §9.9.1 propdef: "Inherited: no". sibling:
        // `text_decoration_non_inherited_child_starts_from_initial` と同じ
        // pattern。
        use crate::property::ZIndexValue;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("z-index: 5"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].z_index, ZIndexValue::Integer(5));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].z_index,
            ZIndexValue::Auto,
            "z-index must not inherit from parent (CSS2 §9.9.1 Inherited: no)"
        );
    }

    #[test]
    fn break_before_wired_through_cascade_from_inline_style() {
        // <p style="break-before: avoid-page"> → ComputedValues.break_before
        // に BreakBetween::AvoidPage が届く。parser → PropertyValue::BreakBefore
        // → apply_value → ComputedValues の end-to-end 疎通 smoke。sibling
        // (box-sizing / z-index) の wire-through pattern を踏襲。
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("break-before: avoid-page"));
        assert_eq!(cv.break_before, BreakBetween::AvoidPage);
    }

    #[test]
    fn break_after_wired_through_cascade_from_inline_style() {
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("break-after: page"));
        assert_eq!(cv.break_after, BreakBetween::Page);
    }

    #[test]
    fn break_inside_wired_through_cascade_from_inline_style() {
        use crate::property::BreakInside;
        let cv = cascade_doc("", "p", Some("break-inside: avoid"));
        assert_eq!(cv.break_inside, BreakInside::Avoid);
    }

    #[test]
    fn break_before_non_inherited_child_starts_from_initial() {
        // CSS Fragmentation Module Level 3 §3.1 propdef: "Inherited: no".
        // sibling: `z_index_non_inherited_child_starts_from_initial` と同じ
        // pattern。
        use crate::property::BreakBetween;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("break-before: page"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].break_before, BreakBetween::Page);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].break_before,
            BreakBetween::Auto,
            "break-before must not inherit from parent (CSS Fragmentation \
             Module Level 3 §3.1 Inherited: no)"
        );
    }

    #[test]
    fn page_break_before_legacy_shorthand_wired_through_cascade_remaps_to_page() {
        // <p style="page-break-before: always"> → ComputedValues.break_before
        // に BreakBetween::Page が届く (CSS Fragmentation Module Level 3 §3.4
        // mapping table: `always` -> `page`, `BreakBetween` doc's "legacy
        // shorthand" section) — end-to-end check that the non-identity remap
        // survives the full parse -> cascade -> ComputedValues pipeline, not
        // just the `property::tests` parser-level check.
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("page-break-before: always"));
        assert_eq!(cv.break_before, BreakBetween::Page);
    }

    #[test]
    fn font_weight_keyword_bold_wired_through_cascade_from_inline_style() {
        // <p style="font-weight: bold"> → ComputedValues.font_weight = 700。
        // parser Ident arm → PropertyValue::FontWeight(Absolute(700)) → apply_value →
        // ComputedValues の end-to-end 疎通 smoke (既存 wire-through test と同じ
        // pattern を踏襲)。
        let cv = cascade_doc("", "p", Some("font-weight: bold"));
        assert_eq!(cv.font_weight, 700.0);
    }

    #[test]
    fn font_weight_keyword_normal_wired_through_cascade_from_inline_style() {
        // <p style="font-weight: normal"> → ComputedValues.font_weight = 400。
        let cv = cascade_doc("", "p", Some("font-weight: normal"));
        assert_eq!(cv.font_weight, 400.0);
    }

    #[test]
    fn font_weight_is_inherited_child_carries_parent_bold() {
        // CSS Fonts 4 §2.2 "Inheritance: Yes"。<p style="font-weight: bold"> の
        // 子 <span> は自身 rule 無しでも parent の 700 を継承する。
        // Verification #7: parent bold + child 未指定 = child 700。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bold"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
        assert_eq!(
            r.computed[span].font_weight, 700.0,
            "font-weight must be inherited (CSS Fonts 4 §2.2 Yes) — \
             parent bold keyword → child inherits 700"
        );
    }

    fn relative_weight_through_cascade(parent_decl: &str, child_decl: &str) -> f32 {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some(parent_decl));
        let span = doc.push_element(p, "span", Some(child_decl));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        r.computed[span].font_weight
    }

    #[test]
    fn font_weight_bolder_lighter_table_all_six_rows() {
        // CSS Fonts 4 §2.2.1 の bolder/lighter table を 6 行 × 2 列すべて直接
        // 検証する。unit 関数を叩くことで cascade test setup に依存せず表の
        // 境界 (半開区間) を網羅する。
        //
        // | inherited w    | bolder | lighter |
        // | w < 100        | 400    | w       |
        // | 100 <= w < 350 | 400    | 100     |
        // | 350 <= w < 550 | 700    | 100     |
        // | 550 <= w < 750 | 900    | 400     |
        // | 750 <= w < 900 | 900    | 700     |
        // | 900 <= w       | w      | 700     |
        let bolder = |w| resolve_relative_weight(FontWeightValue::Bolder, w);
        let lighter = |w| resolve_relative_weight(FontWeightValue::Lighter, w);

        // row 1: w < 100 (lighter = no change)
        assert_eq!(bolder(1.0), 400.0);
        assert_eq!(bolder(99.0), 400.0);
        assert_eq!(lighter(1.0), 1.0);
        assert_eq!(lighter(99.0), 99.0);
        // row 2: 100 <= w < 350
        assert_eq!(bolder(100.0), 400.0);
        assert_eq!(bolder(349.0), 400.0);
        assert_eq!(lighter(100.0), 100.0);
        assert_eq!(lighter(349.0), 100.0);
        // row 3: 350 <= w < 550
        assert_eq!(bolder(350.0), 700.0);
        assert_eq!(bolder(549.0), 700.0);
        assert_eq!(lighter(350.0), 100.0);
        assert_eq!(lighter(549.0), 100.0);
        // row 4: 550 <= w < 750
        assert_eq!(bolder(550.0), 900.0);
        assert_eq!(bolder(749.0), 900.0);
        assert_eq!(lighter(550.0), 400.0);
        assert_eq!(lighter(749.0), 400.0);
        // row 5: 750 <= w < 900
        assert_eq!(bolder(750.0), 900.0);
        assert_eq!(bolder(899.0), 900.0);
        assert_eq!(lighter(750.0), 700.0);
        assert_eq!(lighter(899.0), 700.0);
        // row 6: 900 <= w (bolder = no change)
        assert_eq!(bolder(900.0), 900.0);
        assert_eq!(bolder(1000.0), 1000.0);
        assert_eq!(lighter(900.0), 700.0);
        assert_eq!(lighter(1000.0), 700.0);
    }

    #[test]
    fn font_weight_table_no_change_rows_are_not_clamps() {
        // 表の両端 2 行は "no change" であって clamp ではない。算術近似
        // (`min(w + 300, 900)` / `max(w - 300, 100)`) を書くとここが壊れる。
        // この 2 行は `font-weight` の受理 range を `[1,1000]` に広げて初めて
        // author から到達可能になったため、regression guard として置いている。
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Bolder, 1000.0),
            1000.0,
            "900 <= w row is no-change: bolder(1000) must stay 1000, not clamp to 900"
        );
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Lighter, 50.0),
            50.0,
            "w < 100 row is no-change: lighter(50) must stay 50, not rise to 100"
        );
    }

    #[test]
    fn font_weight_absolute_ignores_inherited_weight() {
        // `<font-weight-absolute>` は継承値と無関係にそのまま computed になる。
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Absolute(250.0), 900.0),
            250.0
        );
    }

    #[test]
    fn resolve_relative_weight_non_finite_inherited_is_asymmetric() {
        let bolder = |w| resolve_relative_weight(FontWeightValue::Bolder, w);
        let lighter = |w| resolve_relative_weight(FontWeightValue::Lighter, w);

        // NaN: `<` 比較は常に false なので両 arm とも catch-all に落ちる。
        // catch-all の中身が違うので結果も違う —
        // `Bolder` の catch-all は `w => w` (900 <= w 行の "no change") な
        // ので NaN がそのまま伝播する。
        // Bolder(NaN) must propagate NaN as-is (catch-all is `w => w`).
        assert!(bolder(f32::NAN).is_nan());
        // `Lighter` の catch-all は `_ => 700.0` なので NaN は 700.0 に丸め
        // られる。
        // Lighter(NaN) must round to 700.0 (catch-all is `_ => 700.0`, not `w => w`).
        assert_eq!(lighter(f32::NAN), 700.0);

        // +Inf: `<` 比較は NaN と同じく常に false なので、同じ catch-all
        // 経路 (NaN と同型の非対称)。
        // Bolder(+Inf) must propagate +Inf as-is.
        assert_eq!(bolder(f32::INFINITY), f32::INFINITY);
        // Lighter(+Inf) must round to 700.0.
        assert_eq!(lighter(f32::INFINITY), 700.0);

        // -Inf: `w < 100.0` の最初の guard に一致するので、Bolder/Lighter
        // どちらも catch-all を経ない唯一の非有限入力 — ただし row 1 に
        // ヒットした後の結果は arm ごとに違う。`Bolder` の row 1 は `w if w <
        // 100.0 => 400.0` なので、有限の `w < 100` と同じく 400.0 に解決
        // される (正常な値)。`Lighter` の row 1 は "no change" arm (`w if w <
        // 100.0 => w`) なので `-Inf` はそのまま伝播する — row にヒットする
        // ことと結果が正常な有限値になることは同じではない。
        // Bolder(-Inf) hits row 1 (w < 100) like any finite w < 100 and
        // resolves to 400.0.
        assert_eq!(bolder(f32::NEG_INFINITY), 400.0);
        // Lighter(-Inf) hits row 1's no-change arm (`w if w < 100.0 => w`) and propagates -Inf.
        assert_eq!(lighter(f32::NEG_INFINITY), f32::NEG_INFINITY);
    }

    #[test]
    fn resolve_relative_font_size_applies_1_2_ratio() {
        assert_eq!(
            resolve_relative_font_size(RelativeFontSize::Larger, 16.0),
            19.2
        );
        assert_eq!(
            resolve_relative_font_size(RelativeFontSize::Smaller, 16.0),
            16.0 / 1.2
        );
        // `×1.2` の後 `÷1.2` は f32 の丸めにより **bit 一致はしない** —
        // spec が round-trip を要求しているわけではなく、単に実装が table
        // lookup ではなく単純 ratio の合成であることの pin。浮動小数誤差の
        // 範囲 (`< 0.0001`) では元の値に戻ることを確認する。
        let round_tripped = resolve_relative_font_size(
            RelativeFontSize::Smaller,
            resolve_relative_font_size(RelativeFontSize::Larger, 16.0),
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (round_tripped - 16.0).abs() < 0.0001,
            "×1.2 の後 ÷1.2 すれば浮動小数誤差の範囲で元に戻るはず: {round_tripped}"
        );
    }

    #[test]
    fn resolved_against_inherited_carries_the_value_without_loss() {
        let inherited = ComputedValues::initial();
        let ctx = ResolveContext::new(inherited.font_size);

        // 解決される側 (payload が変わる) — `bolder` は継承元 400 に対して
        // 700 に解決される。
        let resolved = resolve_against_inherited(
            PropertyValue::FontWeight(FontWeightValue::Bolder),
            &inherited,
            &ctx,
        );
        assert_eq!(
            resolved.as_property_value(),
            &PropertyValue::FontWeight(FontWeightValue::Absolute(700.0)),
            "as_property_value は所有権を取らずに中身を覗けること",
        );
        assert_eq!(
            resolved.into_property_value(),
            PropertyValue::FontWeight(FontWeightValue::Absolute(700.0)),
            "into_property_value は同じ値を消費して取り出せること",
        );

        // pass-through 側 (payload は変わらない) — `Color` はこの関数の対象外
        // なので `v` がそのまま返る。
        let passthrough =
            resolve_against_inherited(PropertyValue::Color(CssColor::BLACK), &inherited, &ctx);
        assert_eq!(
            passthrough.into_property_value(),
            PropertyValue::Color(CssColor::BLACK),
        );

        // `background-position` — added `v @ (...)` pass-through arm (CSS
        // Backgrounds 3 §2.6). This arm is only reachable via the `@page`
        // path (`crate::page::cascade_page`) in practice — the element
        // path goes through `apply_value` directly — so this direct call
        // is this arm's only coverage of the 7 `Background*` variants in
        // that bucket (same shape as the `Color` assertion above, which
        // covers `background_color`'s sibling arm the same way).
        let position = PropertyValue::BackgroundPosition(crate::property::CssPosition {
            horizontal: crate::property::CssPositionOffset::Start(Length::Px(5.0)),
            vertical: crate::property::CssPositionOffset::Start(Length::Px(5.0)),
        });
        let passthrough = resolve_against_inherited(position.clone(), &inherited, &ctx);
        assert_eq!(passthrough.into_property_value(), position);
    }

    #[test]
    fn font_weight_bolder_wired_through_cascade_from_parent_computed() {
        // Verification #3 / #4 / #5。親の **computed** weight に対して
        // resolve される (parse → PropertyValue::FontWeight(Bolder) →
        // apply_value → ComputedValues の end-to-end 疎通)。
        assert_eq!(
            relative_weight_through_cascade("font-weight: 400", "font-weight: bolder"),
            700.0
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 700", "font-weight: bolder"),
            900.0
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 900", "font-weight: bolder"),
            900.0,
            "900 <= w row: bolder is a no-op at 900"
        );
    }

    #[test]
    fn font_weight_lighter_wired_through_cascade_from_parent_computed() {
        // Verification #6。
        assert_eq!(
            relative_weight_through_cascade("font-weight: 100", "font-weight: lighter"),
            100.0,
            "100 <= w < 350 row: lighter(100) = 100"
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 700", "font-weight: lighter"),
            400.0
        );
    }

    #[test]
    fn font_weight_relative_resolves_against_computed_not_literal_parent_value() {
        // Verification #7 の核心。親の declaration は `bold` keyword
        // (literal な spec 値は "bold" であって数値ではない) だが、resolution は
        // 親の **computed** 700 に対して行われる → bolder(700) = 900。
        assert_eq!(
            relative_weight_through_cascade("font-weight: bold", "font-weight: bolder"),
            900.0,
            "parent keyword `bold` must be computed to 700 first, then bolder(700) = 900"
        );
        // 親自身が bolder の場合は連鎖する: 親 = bolder(400 initial) = 700、
        // 子 = bolder(700) = 900。継承値が「親の computed」であることの証明。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bolder"));
        let span = doc.push_element(p, "span", Some("font-weight: bolder"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_weight, 700.0,
            "root-level bolder resolves against the initial 400"
        );
        assert_eq!(
            r.computed[span].font_weight, 900.0,
            "nested bolder must chain off the parent's computed 700, not off 400"
        );
    }

    #[test]
    fn font_weight_relative_is_inherited_as_resolved_absolute() {
        // bolder を解決した親の値は、以降 absolute weight として通常どおり
        // inherit される (computed side に sentinel が漏れない証明)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bolder"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[span].font_weight, 700.0);
    }

    #[test]
    fn font_weight_full_range_wired_through_cascade() {
        // spec range `[1,1000]` の両端が cascade まで届く。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1")).font_weight,
            1.0
        );
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1000")).font_weight,
            1000.0
        );
        // fractional は f32 格上げ以降、丸めずそのまま computed value まで届く
        // (旧実装は round-half-away-from-zero で 101 に
        // 丸めていた)。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 100.5")).font_weight,
            100.5
        );
    }

    #[test]
    fn font_weight_wpt_font_weight_computed_150_25() {
        // WPT css/css-fonts/parsing/font-weight-computed.html:
        // `test_computed_value('font-weight', '150.25')` を cascade を経由した
        // computed side で check する (parse 側の同値 check は
        // `crate::property::tests::font_weight_wpt_font_weight_computed_150_25`)。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 150.25")).font_weight,
            150.25
        );
    }

    #[test]
    fn bolder_lighter_resolve_against_unrounded_fractional_parent_weight() {
        // 実際に起きていた origin failure scenario (3 件、以下そのまま pin)。
        // 丸めが `u16` computed 表現に
        // 起因していた頃は、350 単位の relative-weight table 行選択そのものが
        // ずれていた:
        //
        // - `p { font-weight: 349.5 } span { font-weight: bolder }`
        //   spec: 349.5 は `100 <= w < 350` 行 → bolder = **400**
        //   旧実装: parse が 350 に丸め → `350 <= w < 550` 行 → **700** (誤り)
        // - `549.5` + `bolder`: spec **700** / 旧実装 **900** (誤り)
        // - `749.5` + `lighter`: spec **400** / 旧実装 **700** (誤り)
        //
        // payload / `ComputedValues.font_weight` を `f32` に格上げしたことで
        // 丸め自体が無くなり、以下は spec どおりの行に解決される。
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            relative_weight_through_cascade("font-weight: 349.5", "font-weight: bolder"),
            400.0,
            "349.5 is in the `100 <= w < 350` row, not `350 <= w < 550`"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            relative_weight_through_cascade("font-weight: 549.5", "font-weight: bolder"),
            700.0,
            "549.5 is in the `350 <= w < 550` row, not `550 <= w < 750`"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            relative_weight_through_cascade("font-weight: 749.5", "font-weight: lighter"),
            400.0,
            "749.5 is in the `550 <= w < 750` row, not `750 <= w < 900`"
        );
    }

    #[test]
    fn multiple_elements_each_carry_own_running_template() {
        // 複数 element がそれぞれ異なる running(name) を持つ →
        // per-node で seed が独立に格納される (per-document concat は下流責務)。
        use crate::computed::RunningTemplate;
        let mut doc = TestDoc::new();
        let h = doc.push_element(0, "header", Some("position: running(hdr)"));
        let f = doc.push_element(0, "footer", Some("position: running(ftr)"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h].running_templates,
            vec![RunningTemplate {
                name: SmolStr::new("hdr")
            }]
        );
        assert_eq!(
            r.computed[f].running_templates,
            vec![RunningTemplate {
                name: SmolStr::new("ftr")
            }]
        );
    }

    #[test]
    fn cascade_shares_content_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        // 攻撃 vector そのものの縮小版: universal selector + 単一 literal payload。
        doc.push_text(s, r#"* { content: "shared payload" }"#);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // Sanity: 両 element とも content が届いている。
        assert_eq!(r.computed[p1].content.len(), 1);
        assert_eq!(r.computed[p2].content.len(), 1);
        // Regression assert: Arc pointer identity で shallow-shared を証明。
        // deep-clone 復活時は underlying alloc が別、ptr_eq = false → fail。
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].content, &r.computed[p2].content),
            "cascade must Arc-share content across universal-selector matches \
             (SEC HIGH DoS regression)"
        );
    }

    #[test]
    fn cascade_shares_string_set_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"* { string-set: k "shared payload" }"#);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].string_set.len(), 1);
        assert_eq!(r.computed[p2].string_set.len(), 1);
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].string_set, &r.computed[p2].string_set),
            "cascade must Arc-share string_set across universal-selector matches"
        );
    }

    #[test]
    fn cascade_shares_counter_reset_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        // 攻撃 vector そのものの縮小版: universal selector + 3-name payload。
        doc.push_text(s, "* { counter-reset: c0 c1 c2 }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // Sanity: 両 element とも counter_reset が届いている。
        assert_eq!(r.computed[p1].counter_reset.len(), 3);
        assert_eq!(r.computed[p2].counter_reset.len(), 3);
        // Regression assert: Arc pointer identity で shallow-shared を証明。
        // deep-clone 復活時は underlying alloc が別、ptr_eq = false → fail。
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].counter_reset, &r.computed[p2].counter_reset),
            "cascade must Arc-share counter_reset across universal-selector matches \
             (SEC HIGH DoS regression)"
        );
    }

    #[test]
    fn cascade_shares_counter_increment_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "* { counter-increment: c0 c1 c2 }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].counter_increment.len(), 3);
        assert_eq!(r.computed[p2].counter_increment.len(), 3);
        assert!(
            std::sync::Arc::ptr_eq(
                &r.computed[p1].counter_increment,
                &r.computed[p2].counter_increment
            ),
            "cascade must Arc-share counter_increment across universal-selector matches"
        );
    }

    #[test]
    fn cascade_shares_counter_set_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "* { counter-set: c0 c1 c2 }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].counter_set.len(), 3);
        assert_eq!(r.computed[p2].counter_set.len(), 3);
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].counter_set, &r.computed[p2].counter_set),
            "cascade must Arc-share counter_set across universal-selector matches"
        );
    }

    #[test]
    fn resolve_inheritance_uses_initial_arc_for_non_inherited_counter_on_child() {
        let mut doc = TestDoc::new();
        // 親 <parent> に counter-reset を付け、child <child> は counter rule 無し。
        let parent = doc.push_element(0, "parent", Some("counter-reset: c 1"));
        let child = doc.push_element(parent, "child", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // 親は counter_reset を持つ、child は non-inherited のため empty。
        assert_eq!(r.computed[parent].counter_reset.len(), 1);
        assert!(r.computed[child].counter_reset.is_empty());
        // child の counter_reset Arc は shared empty slot と ptr_eq (empty Arc
        // slot 再利用の behavioral proxy)。ここが false になる regression:
        // (a) inherit_from が parent の Arc をそのまま渡してしまう
        // (b) inherit_from が per-node `Arc::new(Vec::new())` を alloc する
        // どちらも上記の memory 目標を破る。
        let shared = crate::property::empty_counter_entries();
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[child].counter_reset, &shared),
            "child counter_reset must point to shared empty Arc slot \
             (non-inherited short-circuit-equivalent canary)"
        );
    }

    #[test]
    fn initial_empty_content_and_string_set_share_arc_slot() {
        let mut doc = TestDoc::new();
        // rule なし、element 2 個 (両者 empty content / string_set)。
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // どちらも empty (initial)。
        assert!(r.computed[p1].content.is_empty());
        assert!(r.computed[p2].content.is_empty());
        assert!(r.computed[p1].string_set.is_empty());
        assert!(r.computed[p2].string_set.is_empty());
        // Shared empty Arc slot を指しているので ptr_eq = true。
        // Arc::new(Vec::new()) を initial/inherit_from で直に呼ぶ regression が
        // 出た瞬間ここが false になり、DoS fix が memory alloc regression に
        // 転じたことを検知する。
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].content, &r.computed[p2].content),
            "empty content must reuse shared Arc slot — per-node empty Arc \
             allocation regression detected (side-effect canary)"
        );
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].string_set, &r.computed[p2].string_set),
            "empty string_set must reuse shared Arc slot (side-effect canary)"
        );
    }

    #[test]
    fn initial_font_family_shares_arc_slot_across_independent_cascade_runs() {
        let mut doc_a = TestDoc::new();
        let a = doc_a.push_element(0, "p", None);
        let tree_a = build_rule_tree(&doc_a);
        let r_a = cascade(&doc_a, &tree_a).expect("cascade Ok");

        let mut doc_b = TestDoc::new();
        let b = doc_b.push_element(0, "p", None);
        let tree_b = build_rule_tree(&doc_b);
        let r_b = cascade(&doc_b, &tree_b).expect("cascade Ok");

        assert_eq!(r_a.computed[a].font_family.len(), 1);
        assert_eq!(r_b.computed[b].font_family.len(), 1);
        assert!(
            std::sync::Arc::ptr_eq(&r_a.computed[a].font_family, &r_b.computed[b].font_family),
            "initial font_family must reuse the shared `initial_font_family()` Arc \
             slot across independent cascade() runs"
        );
    }

    #[test]
    fn cascade_shares_font_family_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "* { font-family: Arial, sans-serif }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].font_family.len(), 2);
        assert_eq!(r.computed[p2].font_family.len(), 2);
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].font_family, &r.computed[p2].font_family),
            "cascade must Arc-share font_family across universal-selector matches"
        );
    }

    #[test]
    fn resolve_inheritance_shares_font_family_arc_from_parent_when_child_has_no_declaration() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "parent", Some("font-family: Georgia, serif"));
        let child = doc.push_element(parent, "child", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[parent].font_family.len(), 2);
        assert_eq!(r.computed[child].font_family.len(), 2);
        assert!(
            std::sync::Arc::ptr_eq(
                &r.computed[parent].font_family,
                &r.computed[child].font_family
            ),
            "child with no font-family declaration must inherit the parent's \
             Arc by identity, not a re-cloned Vec"
        );
    }

    #[test]
    fn padding_shorthand_wired_through_cascade_from_inline_style() {
        // Verification #7: `padding: 10px 5%` → 2-value form expansion で
        // top/bottom=10px, left/right=5% を pin。counter-* / content / string_set
        // wire-through pattern を踏襲。
        use crate::property::Sides;
        let cv = cascade_doc("", "div", Some("padding: 10px 5%"));
        assert_eq!(
            cv.padding,
            Sides {
                top: ComputedLengthPercentage::Px(10.0),
                right: ComputedLengthPercentage::Percent(5.0),
                bottom: ComputedLengthPercentage::Px(10.0),
                left: ComputedLengthPercentage::Percent(5.0),
            }
        );
    }

    #[test]
    fn padding_longhand_wired_through_cascade_from_inline_style() {
        // 4 longhand も端から端まで届くことを smoke で pin。
        use crate::property::Sides;
        let cv = cascade_doc(
            "",
            "div",
            Some("padding-top: 1px; padding-right: 2px; padding-bottom: 3px; padding-left: 4px"),
        );
        assert_eq!(
            cv.padding,
            Sides {
                top: ComputedLengthPercentage::Px(1.0),
                right: ComputedLengthPercentage::Px(2.0),
                bottom: ComputedLengthPercentage::Px(3.0),
                left: ComputedLengthPercentage::Px(4.0),
            }
        );
    }

    #[test]
    fn padding_is_non_inherited_child_starts_from_initial_zero() {
        // Verification #7 の後半: <div style="padding: 10px 5%"> の子 <span> は
        // 自身 rule がなく padding は initial (Sides::all(0px))。
        // sibling: string_set / content / display / counter-* / running_templates
        // の non-inheritance test と同じ shape。
        use crate::property::Sides;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("padding: 10px 5%"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // 親は shorthand 由来の値を持つ。
        assert_eq!(
            r.computed[div].padding,
            Sides {
                top: ComputedLengthPercentage::Px(10.0),
                right: ComputedLengthPercentage::Percent(5.0),
                bottom: ComputedLengthPercentage::Px(10.0),
                left: ComputedLengthPercentage::Percent(5.0),
            }
        );
        // 子は inherit_from 経由で initial (Sides::all(0px)) — 継承しない。
        assert_eq!(
            r.computed[span].padding,
            Sides::all(ComputedLengthPercentage::Px(0.0))
        );
    }

    #[test]
    fn padding_negative_declaration_dropped_at_cascade() {
        // spec (CSS Box 3) §4.1 negative reject の end-to-end smoke: cascade まで負値が
        // 到達せず、initial (0) が残る。property.rs test は parse_value 単体
        // の drop、本 test は rule.rs → cascade の一貫 drop を pin。
        use crate::property::Sides;
        let cv = cascade_doc("", "div", Some("padding-top: -5px"));
        // 負値 → declaration drop → padding は cascade 未 override → initial 0 が残る。
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    #[test]
    fn padding_shorthand_then_longhand_longhand_wins() {
        // CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand>: shorthand は
        // parse-time で longhand に expand してから cascade する。
        // `padding: 10px; padding-top: 5px;` →
        // top=5, others=10 (source-order-independent、spec-correct)。
        // (margin で実装済みの parse-time expansion model に migrate 済み)
        let cv = cascade_doc("", "div", Some("padding: 10px; padding-top: 5px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    #[test]
    fn padding_longhand_then_shorthand_shorthand_wins() {
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で check: `padding-top: 5px; padding: 10px;`
        // → 全 side = 10px (後段 shorthand が top も含めて上書き)。
        let cv = cascade_doc("", "div", Some("padding-top: 5px; padding: 10px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    #[test]
    fn margin_shorthand_wired_through_cascade_two_value_expansion() {
        // Verification 6-a: `<div style="margin: 10px 20px">` → ComputedValues.margin
        // に top=10, right=20, bottom=10, left=20 が届く。
        // parser → parse_declaration_block (shorthand expand) → 4 longhand
        // PropertyValue → apply_value → ComputedValues の end-to-end 疎通 smoke。
        let cv = cascade_doc("", "div", Some("margin: 10px 20px"));
        assert_eq!(
            cv.margin,
            Sides {
                top: ComputedLengthPercentageOrAuto::Px(10.0),
                right: ComputedLengthPercentageOrAuto::Px(20.0),
                bottom: ComputedLengthPercentageOrAuto::Px(10.0),
                left: ComputedLengthPercentageOrAuto::Px(20.0),
            }
        );
    }

    #[test]
    fn margin_longhand_wired_through_cascade_single_side() {
        // longhand direct path — `<p style="margin-left: 2em">` → left = Em(2)、
        // 他 side は initial (0)。
        let cv = cascade_doc("", "p", Some("margin-left: 2em"));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(32.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_auto_wired_through_cascade_horizontal_centering() {
        // Verification 5: `margin: 0 auto` (block-level horizontal centering の
        // 慣用形) が全 4 side に正しく落ちる。cascade で LengthOrAuto::Auto の
        // wire-through を pin。
        let cv = cascade_doc("", "div", Some("margin: 0px auto"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn margin_shorthand_then_longhand_later_longhand_wins() {
        // spec (CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort>): 同一 declaration
        // block 内で shorthand + longhand が declared された場合、後方
        // declaration が同 rank/spec/order で勝つ。
        // `margin: 0px; margin-top: 10px;` → top=10, others=0。
        //
        // 本 test は本 architecture の load-bearing case: expansion 前 shorthand
        // を単一 key で cascade してしまうと、`PropertyKey` 宣言順では `Margin`
        // が `MarginTop` より後に来るため `margin` が必ず後勝ちし top=0 に
        // 上書きされる (spec と逆)。expand_shorthand_into が parse-time で longhand
        // 化するため per-key の cascade winner が top=10 に確定する。
        let cv = cascade_doc("", "div", Some("margin: 0px; margin-top: 10px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_longhand_then_shorthand_later_shorthand_wins() {
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で check: `margin-top: 10px; margin: 0px;`
        // → 全 side = 0px (後段 shorthand が top も含めて上書き)。
        // expand_shorthand_into の 4 longhand 展開が source_order を保持したまま
        // cascade に届き、後段が per-side 勝ち抜けする証拠。
        let cv = cascade_doc("", "div", Some("margin-top: 10px; margin: 0px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_non_inherited_child_starts_from_initial() {
        // Verification 6-b: CSS Box 3 §3.1 "Inherited: no"。<div style="margin:
        // 20px"> の子 <span> は自身 rule 無しで margin = initial (0 spread)。
        // sibling: display / string_set / content non-inherited と同 shape。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("margin: 20px"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].margin,
            Sides::all(ComputedLengthPercentageOrAuto::Px(20.0))
        );
        assert_eq!(
            r.computed[span].margin,
            Sides::all(ComputedLengthPercentageOrAuto::Px(0.0)),
            "margin must not inherit from parent"
        );
    }

    #[test]
    fn margin_negative_length_accepted() {
        // Task Non-goals: negative margin は spec-valid (§3.1)、cascade の end-to-end
        // で受理されることを check (parser 側 check `margin_side_accepts_negative_length`
        // と complementary、下流 layout 側で negative 意味付け)。
        let cv = cascade_doc("", "div", Some("margin-top: -5px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(-5.0));
    }

    #[test]
    fn padding_inline_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("padding-inline-start: 5px; padding-inline-end: 10px"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        // untouched block axis stays at initial (0).
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn padding_block_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("padding-block-start: 5px; padding-block-end: 10px"),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn margin_inline_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "p",
            Some("margin-inline-start: 5px; margin-inline-end: auto"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_block_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "p",
            Some("margin-block-start: auto; margin-block-end: 5px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn padding_inline_shorthand_one_value_wired_through_cascade() {
        // 1-value form spreads to both start/end (CSS Logical Properties and
        // Values 1 §4.4 "If only one value is given, it applies to both the
        // start and end edges").
        let cv = cascade_doc("", "div", Some("padding-inline: 12px"));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(12.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(12.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn padding_inline_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("padding-inline: 5px 10px"));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
    }

    #[test]
    fn padding_block_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("padding-block: 5px 10px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn margin_inline_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("margin-inline: 5px auto"));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_block_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("margin-block: auto 5px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_inline_start_and_margin_left_compete_on_the_same_cascade_key() {
        // Physical `margin-left` and logical `margin-inline-start` are
        // fixed-mapped to the exact same `PropertyValue::MarginLeft` variant
        // at parse time (`PropertyValue::PaddingInline` doc's "なぜ 8
        // longhand が専用 variant を持たないか" section) — so, per CSS
        // Logical Properties and Values 1 §4 ("corresponding flow-relative
        // and physical properties are paired"), the two compete for the
        // *same* cascade winner, with the later declaration winning (CSS
        // Cascading L4 §6.1 "Order of Appearance"). Same shape as
        // `margin_shorthand_then_longhand_later_longhand_wins`.
        let later_logical_wins = cascade_doc(
            "",
            "div",
            Some("margin-left: 1px; margin-inline-start: 2px"),
        );
        assert_eq!(
            later_logical_wins.margin.left,
            ComputedLengthPercentageOrAuto::Px(2.0)
        );
        let later_physical_wins = cascade_doc(
            "",
            "div",
            Some("margin-inline-start: 2px; margin-left: 1px"),
        );
        assert_eq!(
            later_physical_wins.margin.left,
            ComputedLengthPercentageOrAuto::Px(1.0)
        );
    }

    #[test]
    fn margin_inline_shorthand_important_beats_later_normal_physical_longhand() {
        // CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim:
        // "Declaring a shorthand property to be !important is equivalent to
        // declaring all of its sub-properties to be !important." —
        // `crate::rule::expand_shorthand_into`'s `important` threading
        // (`expand_margin_inline` etc. each take and propagate an
        // `important: bool`) must hold for the 2-value logical shorthands
        // too, not just the physical `margin`/`padding` shorthands this same
        // guarantee already covers.
        //
        // Source order alone would make the later `margin-left: 20px` win
        // (CSS Cascading L4 §6.1 "Order of Appearance"), but Origin and
        // Importance rank higher than order (§6.1) — so this only comes out
        // to 5px if the `!important` flag actually survived the `margin-inline`
        // → `MarginLeft`/`MarginRight` fan-out.
        let cv = cascade_doc(
            "",
            "div",
            Some("margin-inline: 5px !important; margin-left: 20px"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(5.0));
    }

    #[test]
    fn margin_inline_shorthand_three_values_declaration_dropped() {
        // End-to-end sibling of `margin_shorthand_five_values_declaration_dropped`
        // for the 2-value logical shorthand: `property::tests`'s
        // `margin_inline_shorthand_leaves_extra_values_for_caller_exhausted_check`
        // pins the bare `parse_value`-level behavior (2 values consumed,
        // 3rd left unconsumed, `Some` still returned) and *claims* the
        // caller's `expect_exhausted` drops the whole declaration — this
        // test is the actual end-to-end confirmation of that claim, through
        // `cascade_doc`'s real stylesheet-parse path (same shape as
        // `padding_shorthand_rejects_any_negative_value`'s leftover-token
        // drop, but via a real `<div style>` rather than a hand-built
        // `Parser`).
        let cv = cascade_doc("", "div", Some("margin-inline: 5px 10px 15px"));
        // Declaration dropped entirely → margin stays at initial (0), not
        // the would-be start/end pair (5px/10px) the 2-value prefix alone
        // would produce.
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn height_wired_through_cascade_from_inline_style() {
        // <div style="height: 100px"> → ComputedValues.height に
        // LengthOrAuto::Length(Length::Px(100)) が届く。parser →
        // PropertyValue::Height → apply_value → ComputedValues の end-to-end
        // 疎通 smoke (margin / padding wire-through pattern を踏襲)。
        let cv = cascade_doc("", "div", Some("height: 100px"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn height_auto_wired_through_cascade() {
        // `height: auto` は spec initial (§3.1.1) だが cascade winner として
        // declaration が到達した場合の受理 pattern を明示 check
        // (`static_position_wins_over_running_via_source_order` 系の pattern、
        // parser の auto ident branch と apply_value の LengthOrAuto::Auto 経路
        // が疎通することを保証)。
        let cv = cascade_doc("", "div", Some("height: auto"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn height_percentage_wired_through_cascade() {
        // `height: 50%` の end-to-end 疎通。resolve (containing block % → 実寸)
        // は下流責務、cascade は authored value をそのまま保持することを pin。
        let cv = cascade_doc("", "div", Some("height: 50%"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Percent(50.0));
    }

    #[test]
    fn height_non_inherited_child_starts_from_initial() {
        // Verification 6 (task doc): CSS Sizing 3 §3.1.1 "Inherited: no"。
        // <div style="height: 100px"> の子 <span> は自身 rule 無しで
        // height = initial (`LengthOrAuto::Auto`)。sibling: margin / padding
        // / display / string_set / content non-inherited と同 shape。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("height: 100px"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].height,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(
            r.computed[span].height,
            ComputedLengthPercentageOrAuto::Auto,
            "height must not inherit from parent (§3.1.1 Inherited: no)"
        );
    }

    #[test]
    fn height_negative_length_rejected_at_parse_time() {
        // Non-goal (a) spec-invalid: `height: -10px` は grammar `[0,∞]` 違反、
        // declaration 段で drop → cascade に届かず、height は initial (Auto) の
        // まま。parser 側 check (`height_rejects_negative_length`) と complementary
        // な end-to-end 挙動を確認。
        let cv = cascade_doc("", "div", Some("height: -10px"));
        assert_eq!(
            cv.height,
            ComputedLengthPercentageOrAuto::Auto,
            "negative height declaration must be dropped; height stays at initial"
        );
    }

    #[test]
    fn apply_value_direct_counter_reset_inherit() {
        // `counter-reset: revert`-class staging marker — never produced by
        // the element parser's normal `counter-reset: <counter-name>? ...`
        // grammar, but `apply_value` must still reset `target.counter_reset`
        // to the empty entries sentinel if an internal caller supplies one.
        let mut cv = SpecifiedValues::initial();
        cv.counter_reset = std::sync::Arc::new(vec![(SmolStr::new("foo"), 1)]);
        apply_value(PropertyValue::CounterResetInherit, &mut cv);
        assert_eq!(cv.counter_reset, empty_counter_entries());
    }

    #[test]
    fn apply_value_direct_position_sticky_and_fixed() {
        let mut cv = SpecifiedValues::initial();
        apply_value(PropertyValue::Position(PositionValue::Sticky), &mut cv);
        assert_eq!(cv.position, PositionValue::Sticky);
        apply_value(PropertyValue::Position(PositionValue::Fixed), &mut cv);
        assert_eq!(cv.position, PositionValue::Fixed);
    }

    #[test]
    fn apply_value_direct_padding_shorthand_direct_assign() {
        let mut cv = SpecifiedValues::initial();
        let sides = Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        };
        apply_value(PropertyValue::Padding(sides), &mut cv);
        assert_eq!(cv.padding, sides);
    }

    #[test]
    fn apply_value_direct_margin_inherit_marker_is_panic_free() {
        // Page-only inherit markers are never produced by the element
        // parser; `apply_value` must stay a no-op (not panic) if an
        // internal caller supplies one.
        let mut cv = SpecifiedValues::initial();
        let before = cv.margin;
        apply_value(PropertyValue::MarginInherit, &mut cv);
        assert_eq!(cv.margin, before);
    }

    #[test]
    fn apply_value_direct_calc_length_percentage_keys() {
        let value = CalcLengthPercentage {
            percent: 50.0,
            px: 10.0,
        };
        let expect_calc = LengthOrAuto::Calc(value);
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Height,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.height, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MaxWidth,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.max_width, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MaxHeight,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.max_height, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MinWidth,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.min_width, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MinHeight,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.min_height, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MinBlockSize,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.min_block_size, Some(expect_calc));
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Top,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.top, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Right,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.right, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Bottom,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.bottom, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Left,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.left, expect_calc);

        // Non-matching key: the inner match's wildcard fallback is a no-op
        // (this `key` has no `LengthOrAuto`-shaped field to project into).
        let before = cv.clone();
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Color,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv, before);
    }

    #[test]
    fn apply_value_direct_inset_longhands() {
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Top(LengthOrAuto::Length(Length::Px(1.0))),
            &mut cv,
        );
        assert_eq!(cv.top, LengthOrAuto::Length(Length::Px(1.0)));
        apply_value(
            PropertyValue::Right(LengthOrAuto::Length(Length::Px(2.0))),
            &mut cv,
        );
        assert_eq!(cv.right, LengthOrAuto::Length(Length::Px(2.0)));
        apply_value(
            PropertyValue::Bottom(LengthOrAuto::Length(Length::Px(3.0))),
            &mut cv,
        );
        assert_eq!(cv.bottom, LengthOrAuto::Length(Length::Px(3.0)));
    }

    #[test]
    fn apply_value_direct_border_radius_corner_longhands() {
        let mut cv = SpecifiedValues::initial();
        apply_value(PropertyValue::BorderRadiusTopLeft(Length::Px(1.0)), &mut cv);
        assert_eq!(cv.border_radius.top_left, Length::Px(1.0));
        apply_value(
            PropertyValue::BorderRadiusTopRight(Length::Px(2.0)),
            &mut cv,
        );
        assert_eq!(cv.border_radius.top_right, Length::Px(2.0));
        apply_value(
            PropertyValue::BorderRadiusBottomRight(Length::Px(3.0)),
            &mut cv,
        );
        assert_eq!(cv.border_radius.bottom_right, Length::Px(3.0));
        apply_value(
            PropertyValue::BorderRadiusBottomLeft(Length::Px(4.0)),
            &mut cv,
        );
        assert_eq!(cv.border_radius.bottom_left, Length::Px(4.0));
    }

    #[test]
    fn apply_value_direct_outline_and_offset() {
        let mut cv = SpecifiedValues::initial();
        let outline = Outline {
            width: Length::Px(3.0),
            style: OutlineStyle::Dotted,
            color: OutlineColor::CurrentColor,
        };
        apply_value(PropertyValue::Outline(outline), &mut cv);
        assert_eq!(cv.outline, outline);
        apply_value(PropertyValue::OutlineOffset(Length::Px(5.0)), &mut cv);
        assert_eq!(cv.outline_offset, Length::Px(5.0));
    }

    #[test]
    fn apply_value_direct_grid_area_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::GridArea(area)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 4 `GridRowStart`/`GridColumnStart`/
        // `GridRowEnd`/`GridColumnEnd` longhands before `apply_value` ever
        // sees it) — not a safety net, a canary.
        let mut cv = SpecifiedValues::initial();
        let area = GridAreaShorthand {
            row_start: GridLineValue::Line(1),
            column_start: GridLineValue::Line(2),
            row_end: GridLineValue::Line(3),
            column_end: GridLineValue::Line(4),
        };
        apply_value(PropertyValue::GridArea(area.clone()), &mut cv);
        assert_eq!(cv.grid_row_start, area.row_start);
        assert_eq!(cv.grid_column_start, area.column_start);
        assert_eq!(cv.grid_row_end, area.row_end);
        assert_eq!(cv.grid_column_end, area.column_end);
    }

    #[test]
    fn apply_value_direct_grid_shorthand_fall_through() {
        // Sibling of `apply_value_direct_grid_area_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Grid(shorthand)` arm is
        // unreachable via the cascade path for the same reason — not a
        // safety net, a canary that also checks the shorthand's documented
        // sub-property reset behavior.
        let mut cv = SpecifiedValues::initial();
        cv.grid_auto_flow = GridAutoFlowValue::Column;
        cv.grid_row_start = GridLineValue::Line(9);
        let shorthand = GridShorthand {
            rows: GridTemplateTracks::None,
            columns: GridTemplateTracks::None,
        };
        apply_value(PropertyValue::Grid(shorthand), &mut cv);
        assert_eq!(cv.grid_template_rows, GridTemplateTracks::None);
        assert_eq!(cv.grid_template_columns, GridTemplateTracks::None);
        assert_eq!(cv.grid_template_areas, GridTemplateAreasValue::None);
        assert_eq!(cv.grid_auto_columns, initial_grid_auto_track_list());
        assert_eq!(cv.grid_auto_rows, initial_grid_auto_track_list());
        assert_eq!(cv.grid_auto_flow, GridAutoFlowValue::Row);
        assert_eq!(cv.grid_row_start, GridLineValue::Auto);
        assert_eq!(cv.grid_row_end, GridLineValue::Auto);
        assert_eq!(cv.grid_column_start, GridLineValue::Auto);
        assert_eq!(cv.grid_column_end, GridLineValue::Auto);
    }

    #[test]
    fn apply_value_direct_border_style_width_color_shorthand_fall_through() {
        // Sibling of `apply_value_direct_border_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::BorderStyle`/`BorderWidth`/
        // `BorderColor` arms are unreachable via the cascade path
        // (`expand_shorthand_into` expands each to its 4 side longhands
        // before `apply_value` ever sees it) — not a safety net, a canary.
        let mut cv = SpecifiedValues::initial();
        let styles = Sides {
            top: BorderStyle::Solid,
            right: BorderStyle::Dashed,
            bottom: BorderStyle::Dotted,
            left: BorderStyle::Double,
        };
        apply_value(PropertyValue::BorderStyle(styles), &mut cv);
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
        assert_eq!(cv.border.right.style, BorderStyle::Dashed);
        assert_eq!(cv.border.bottom.style, BorderStyle::Dotted);
        assert_eq!(cv.border.left.style, BorderStyle::Double);

        let widths = Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        };
        apply_value(PropertyValue::BorderWidth(widths), &mut cv);
        assert_eq!(cv.border.top.width, Length::Px(1.0));
        assert_eq!(cv.border.right.width, Length::Px(2.0));
        assert_eq!(cv.border.bottom.width, Length::Px(3.0));
        assert_eq!(cv.border.left.width, Length::Px(4.0));

        let colors = Sides {
            top: BorderColor::CurrentColor,
            right: BorderColor::Resolved(CssColor::BLACK),
            bottom: BorderColor::CurrentColor,
            left: BorderColor::Resolved(CssColor::TRANSPARENT),
        };
        apply_value(PropertyValue::BorderColor(colors), &mut cv);
        assert_eq!(cv.border.top.color, BorderColor::CurrentColor);
        assert_eq!(
            cv.border.right.color,
            BorderColor::Resolved(CssColor::BLACK)
        );
        assert_eq!(cv.border.bottom.color, BorderColor::CurrentColor);
        assert_eq!(
            cv.border.left.color,
            BorderColor::Resolved(CssColor::TRANSPARENT)
        );
    }

    #[test]
    fn apply_value_direct_page_named() {
        use crate::Atom;
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Page(PageValue::Named(Atom::from("chapter"))),
            &mut cv,
        );
        assert_eq!(cv.page, PageValue::Named(Atom::from("chapter")));
    }

    #[test]
    fn resolve_inheritance_grows_undersized_output_vectors() {
        // Defensive safety net: `cascade()`'s normal pre-allocation always
        // sizes `out`/`non_ua_margin_sides`/`authored_writing_modes` to
        // `dom.node_count()` before calling `resolve_inheritance`, so this
        // resize path is never exercised end-to-end. A direct call with
        // deliberately undersized (empty) vectors verifies the safety net
        // actually grows them instead of panicking on out-of-bounds writes.
        let mut doc = TestDoc::new();
        let e = doc.push_element(0, "div", None);
        let id = StyleNodeId(e as u64);
        let cascaded = CascadedArena::new();
        let mut out: Vec<ComputedValues> = Vec::new();
        let mut non_ua_margin_sides: Vec<Sides<bool>> = Vec::new();
        let mut authored_writing_modes: Vec<Option<WritingMode>> = Vec::new();
        let mut page_values = vec![PageValue::Auto; doc.node_count()];
        let mut pseudo_out = HashMap::new();
        resolve_inheritance(
            &doc,
            id,
            &ComputedValues::initial(),
            &cascaded,
            &mut out,
            &mut non_ua_margin_sides,
            &mut authored_writing_modes,
            &mut page_values,
            &mut pseudo_out,
        );
        assert!(out.len() > e);
        assert!(non_ua_margin_sides.len() > e);
        assert!(authored_writing_modes.len() > e);
    }

    #[test]
    #[should_panic(expected = "root_ctx == None は element 親が居ないことを意味するので")]
    fn resolve_inheritance_panics_when_root_parent_font_size_is_not_initial() {
        // The first stack entry `resolve_inheritance` pushes always has
        // `root_ctx == None` (only `cascade()`'s own `dom.root_id()` entry
        // point does this, and it always pairs `None` with
        // `ComputedValues::initial()`). A direct call that violates that
        // caller-side invariant — a non-initial `parent_computed` on the
        // very first node — must trip the debug_assert_eq guarding it.
        let mut doc = TestDoc::new();
        let e = doc.push_element(0, "div", None);
        let id = StyleNodeId(e as u64);
        let cascaded = CascadedArena::new();
        let mut out = vec![ComputedValues::initial(); doc.node_count()];
        let mut non_ua_margin_sides = vec![Sides::all(false); doc.node_count()];
        let mut authored_writing_modes = vec![None; doc.node_count()];
        let mut page_values = vec![PageValue::Auto; doc.node_count()];
        let mut pseudo_out = HashMap::new();
        let mut non_initial_parent = ComputedValues::initial();
        non_initial_parent.font_size = ComputedLength(999.0);
        resolve_inheritance(
            &doc,
            id,
            &non_initial_parent,
            &cascaded,
            &mut out,
            &mut non_ua_margin_sides,
            &mut authored_writing_modes,
            &mut page_values,
            &mut pseudo_out,
        );
    }

    #[test]
    fn apply_winners_direct_margin_shorthand_marks_all_sides_non_ua() {
        // `apply_winners`'s `non_ua_margin_sides` tracking arm for the
        // `Margin`/`MarginInline`/`MarginBlock` shorthand keys is
        // unreachable via the cascade path (shorthand is expanded to the 4
        // side longhands before candidates are collected) — not a safety
        // net, a canary for the shorthand-payload shape.
        let sides = Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        };
        let candidates: Vec<CascadedDecl> =
            vec![(PropertyValue::Margin(sides), false, Origin::Author, 0, 0)];
        let mut winners: Vec<Option<RankedDecl>> = Vec::new();
        let mut specified = SpecifiedValues::initial();
        let inherited = ComputedValues::initial();
        let custom_properties = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut non_ua_margin_sides = Sides::all(false);
        apply_winners(
            &candidates,
            &mut winners,
            &mut specified,
            &inherited,
            &custom_properties,
            None,
            Some(&mut non_ua_margin_sides),
            None,
        );
        assert!(non_ua_margin_sides.top);
        assert!(non_ua_margin_sides.right);
        assert!(non_ua_margin_sides.bottom);
        assert!(non_ua_margin_sides.left);
    }

    #[test]
    fn apply_winners_direct_border_radius_inherit() {
        let candidates: Vec<CascadedDecl> = vec![(
            PropertyValue::BorderRadiusInherit,
            false,
            Origin::Author,
            0,
            0,
        )];
        let mut winners: Vec<Option<RankedDecl>> = Vec::new();
        let mut specified = SpecifiedValues::initial();
        let mut inherited = ComputedValues::initial();
        inherited.border_radius = ComputedBorderRadius {
            top_left: ComputedLengthPercentage::Percent(10.0),
            top_right: ComputedLengthPercentage::Px(2.0),
            bottom_right: ComputedLengthPercentage::Percent(30.0),
            bottom_left: ComputedLengthPercentage::Px(4.0),
        };
        let custom_properties = CustomPropertyEnvironment::from_map(HashMap::new());
        apply_winners(
            &candidates,
            &mut winners,
            &mut specified,
            &inherited,
            &custom_properties,
            None,
            None,
            None,
        );
        assert_eq!(specified.border_radius.top_left, Length::Percent(10.0));
        assert_eq!(specified.border_radius.top_right, Length::Px(2.0));
        assert_eq!(specified.border_radius.bottom_right, Length::Percent(30.0));
        assert_eq!(specified.border_radius.bottom_left, Length::Px(4.0));
    }

    #[test]
    fn apply_winners_direct_page_value() {
        use crate::Atom;
        let candidates: Vec<CascadedDecl> = vec![(
            PropertyValue::Page(PageValue::Named(Atom::from("chapter"))),
            false,
            Origin::Author,
            0,
            0,
        )];
        let mut winners: Vec<Option<RankedDecl>> = Vec::new();
        let mut specified = SpecifiedValues::initial();
        let inherited = ComputedValues::initial();
        let custom_properties = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut page_value = PageValue::Auto;
        apply_winners(
            &candidates,
            &mut winners,
            &mut specified,
            &inherited,
            &custom_properties,
            Some(&mut page_value),
            None,
            None,
        );
        assert_eq!(page_value, PageValue::Named(Atom::from("chapter")));
    }

    #[test]
    fn apply_value_direct_margin_shorthand_fall_through() {
        // `apply_value` の `PropertyValue::Margin(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 4 longhand に展開する)。**これは
        // safety net ではない** — 万一
        // regression / bypass 経路で到達すると `target.margin = sides` の
        // atomic 上書きが 4 longhand winner を必ず破壊する。到達した時点で
        // 既に bug であり、本 test は arm を直接叩いて `unreachable!` 化 or
        // 空 arm regression を捕捉する canary。
        let mut cv = SpecifiedValues::initial();
        let sides = Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        };
        apply_value(PropertyValue::Margin(sides), &mut cv);
        assert_eq!(cv.margin, sides);
    }

    #[test]
    fn apply_value_direct_margin_inline_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: LengthOrAuto::Length(Length::Px(1.0)),
            end: LengthOrAuto::Length(Length::Px(2.0)),
        };
        apply_value(PropertyValue::MarginInline(pair), &mut cv);
        assert_eq!(cv.margin.left, pair.start);
        assert_eq!(cv.margin.right, pair.end);
        // untouched axis stays at initial (0).
        assert_eq!(cv.margin.top, LengthOrAuto::Length(Length::Px(0.0)));
        assert_eq!(cv.margin.bottom, LengthOrAuto::Length(Length::Px(0.0)));
    }

    #[test]
    fn apply_value_direct_margin_block_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: LengthOrAuto::Length(Length::Px(3.0)),
            end: LengthOrAuto::Length(Length::Px(4.0)),
        };
        apply_value(PropertyValue::MarginBlock(pair), &mut cv);
        assert_eq!(cv.margin.top, pair.start);
        assert_eq!(cv.margin.bottom, pair.end);
        assert_eq!(cv.margin.left, LengthOrAuto::Length(Length::Px(0.0)));
        assert_eq!(cv.margin.right, LengthOrAuto::Length(Length::Px(0.0)));
    }

    #[test]
    fn apply_value_direct_padding_inline_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(6.0),
        };
        apply_value(PropertyValue::PaddingInline(pair), &mut cv);
        assert_eq!(cv.padding.left, pair.start);
        assert_eq!(cv.padding.right, pair.end);
        assert_eq!(cv.padding.top, Length::Px(0.0));
        assert_eq!(cv.padding.bottom, Length::Px(0.0));
    }

    #[test]
    fn apply_value_direct_padding_block_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: Length::Px(7.0),
            end: Length::Px(8.0),
        };
        apply_value(PropertyValue::PaddingBlock(pair), &mut cv);
        assert_eq!(cv.padding.top, pair.start);
        assert_eq!(cv.padding.bottom, pair.end);
        assert_eq!(cv.padding.left, Length::Px(0.0));
        assert_eq!(cv.padding.right, Length::Px(0.0));
    }

    #[test]
    fn apply_value_direct_overflow_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s
        // `PropertyValue::Overflow(pair)` arm is unreachable via the
        // cascade path (`expand_shorthand_into` expands it to the 2
        // `OverflowX`/`OverflowY` longhands before `apply_value` ever
        // sees it) — not a safety net, a canary that catches regression
        // if the arm is ever reached with a stale/wrong pair.
        let mut cv = SpecifiedValues::initial();
        let pair = OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        };
        apply_value(PropertyValue::Overflow(pair), &mut cv);
        assert_eq!(cv.overflow, pair);
    }

    #[test]
    fn apply_value_direct_text_decoration_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::TextDecoration(shorthand)`
        // arm is unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 3 `TextDecorationLine`/`TextDecorationStyle`/
        // `TextDecorationColor` longhands before `apply_value` ever sees
        // it) — not a safety net, a canary that catches regression if the
        // arm is ever reached with a stale/wrong shorthand value.
        let mut cv = SpecifiedValues::initial();
        let shorthand = TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::Resolved(RED),
            thickness: TextDecorationThickness::Auto,
        };
        apply_value(PropertyValue::TextDecoration(shorthand), &mut cv);
        assert_eq!(cv.text_decoration_line, shorthand.line);
        assert_eq!(cv.text_decoration_style, shorthand.style);
        assert_eq!(cv.text_decoration_color, shorthand.color);
    }

    #[test]
    fn border_shorthand_then_longhand_later_longhand_wins() {
        // spec (CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort>): 同一 declaration
        // block 内で shorthand + longhand が declared された場合、後方
        // declaration が同 rank/spec/order で勝つ。
        // `border: 1px solid red; border-top-width: 10px;` →
        // top.width=10、他 side の width=1、top.style=Solid、top.color=red 保持。
        //
        // 本 test は本 architecture の load-bearing case:
        // expansion 前 shorthand を単一 key で cascade してしまうと、`PropertyKey`
        // 宣言順では `Border` が `BorderTopWidth` より後に来るため `border` が
        // 必ず後勝ちし top.width=1 に上書きされる (spec と逆)。expand_shorthand_into
        // が parse-time で 12 longhand 化するため per-key の cascade winner が
        // top.width=10 に確定する (margin 0vv.5 precedent の 12-longhand 版)。
        let cv = cascade_doc(
            "",
            "div",
            Some("border: 1px solid red; border-top-width: 10px"),
        );
        assert_eq!(cv.border.top.width, ComputedLength(10.0));
        assert_eq!(cv.border.right.width, ComputedLength(1.0));
        assert_eq!(cv.border.bottom.width, ComputedLength(1.0));
        assert_eq!(cv.border.left.width, ComputedLength(1.0));
        // style / color は shorthand から expand された値のまま (per-side longhand
        // として cascade winner に居座る)。
        let red = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
        // border.color は `BorderColor` enum、shorthand
        // 由来の author-specified red は `Resolved` variant で cascade に届く。
        assert_eq!(cv.border.top.color, BorderColor::Resolved(red));
        assert_eq!(cv.border.right.style, BorderStyle::Solid);
        assert_eq!(cv.border.left.color, BorderColor::Resolved(red));
    }

    #[test]
    fn border_longhand_then_shorthand_later_shorthand_wins() {
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で check: `border-top-width: 10px; border: 1px solid red;`
        // → top.width も 1px (後段 shorthand が top も含めて上書き)。
        // expand_shorthand_into の 12 longhand 展開が source_order を保持した
        // まま cascade に届き、後段が per-side / per-sub-property
        // 勝ち抜けする証拠 (margin sibling と対称)。
        let cv = cascade_doc(
            "",
            "div",
            Some("border-top-width: 10px; border: 1px solid red"),
        );
        assert_eq!(cv.border.top.width, ComputedLength(1.0));
        assert_eq!(cv.border.right.width, ComputedLength(1.0));
        assert_eq!(cv.border.bottom.width, ComputedLength(1.0));
        assert_eq!(cv.border.left.width, ComputedLength(1.0));
    }

    #[test]
    fn border_non_inherited_child_starts_from_initial() {
        // CSS Backgrounds 3 §3 "Borders" — border-* propdef は "Inherited: no"。
        // <div style="border: 5px solid red"> の子 <span> は自身 rule 無しで
        // border = initial (medium / none / currentcolor)。
        // sibling: margin / padding non-inherited test
        // を踏襲。color は `BorderColor` enum で保持。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("border: 5px solid red"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let red = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        // parent は shorthand から expand された per-side 値。
        assert_eq!(r.computed[div].border.top.width, ComputedLength(5.0));
        assert_eq!(r.computed[div].border.top.style, BorderStyle::Solid);
        assert_eq!(r.computed[div].border.top.color, BorderColor::Resolved(red));
        // child は inherit_from が initial に戻す (non-inherited)。
        assert_eq!(
            r.computed[span].border,
            Sides::all(ComputedBorder {
                // specified initial は `medium` (3px) だが border-style が
                // `none` なので computed width は 0px (CSS Backgrounds 3 §3.3
                // "Computed value: … zero if the border style is `none` or
                // `hidden`")。
                width: ComputedLength::ZERO,
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            "border must not inherit from parent"
        );
    }

    #[test]
    fn apply_value_direct_border_shorthand_fall_through() {
        // `apply_value` の `PropertyValue::Border(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 12 longhand に展開する)。**これは
        // safety net ではない** (margin
        // fall-through と対称) — 万一 regression / bypass 経路で到達すると
        // `target.border = sides` の atomic 上書きが 12 longhand winner を必ず
        // 破壊する。到達した時点で既に bug であり、本 test は arm を直接叩いて
        // `unreachable!` 化 or 空 arm regression を捕捉する canary。
        let mut cv = SpecifiedValues::initial();
        // `BorderColor::CurrentColor` を明示 fixture 化
        // (fall-through arm は payload の shape を保持することを check する)。
        let sides = Sides {
            top: Border {
                width: Length::Px(1.0),
                style: BorderStyle::Solid,
                color: BorderColor::CurrentColor,
            },
            right: Border {
                width: Length::Px(2.0),
                style: BorderStyle::Dashed,
                color: BorderColor::Resolved(CssColor::BLACK),
            },
            bottom: Border {
                width: Length::Px(3.0),
                style: BorderStyle::Dotted,
                color: BorderColor::CurrentColor,
            },
            left: Border {
                width: Length::Px(4.0),
                style: BorderStyle::Double,
                color: BorderColor::Resolved(CssColor::TRANSPARENT),
            },
        };
        apply_value(PropertyValue::Border(sides), &mut cv);
        assert_eq!(cv.border, sides);
    }

    #[test]
    fn apply_value_direct_flex_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Flex(f)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 3 `FlexGrow`/`FlexShrink`/`FlexBasis`
        // longhands before `apply_value` ever sees it) — not a safety
        // net, a canary that catches regression if the arm is ever
        // reached with a stale/wrong shorthand payload.
        use crate::property::{FlexBasisValue, FlexShorthand};
        let mut cv = SpecifiedValues::initial();
        let f = FlexShorthand {
            grow: 2.0,
            shrink: 3.0,
            basis: FlexBasisValue::Length(Length::Px(10.0)),
        };
        apply_value(PropertyValue::Flex(f), &mut cv);
        assert_eq!(cv.flex_grow, 2.0);
        assert_eq!(cv.flex_shrink, 3.0);
        assert_eq!(cv.flex_basis, FlexBasisValue::Length(Length::Px(10.0)));
    }

    #[test]
    fn apply_value_direct_flex_flow_shorthand_fall_through() {
        // Sibling of `apply_value_direct_flex_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::FlexFlow(f)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `FlexDirection`/`FlexWrap` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{FlexDirectionValue, FlexFlow, FlexWrapValue};
        let mut cv = SpecifiedValues::initial();
        let f = FlexFlow {
            direction: FlexDirectionValue::RowReverse,
            wrap: FlexWrapValue::Wrap,
        };
        apply_value(PropertyValue::FlexFlow(f), &mut cv);
        assert_eq!(cv.flex_direction, FlexDirectionValue::RowReverse);
        assert_eq!(cv.flex_wrap, FlexWrapValue::Wrap);
    }

    #[test]
    fn apply_value_direct_gap_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Gap(g)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `RowGap`/`ColumnGap` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{GapShorthand, LengthOrNormal};
        let mut cv = SpecifiedValues::initial();
        let g = GapShorthand {
            row: LengthOrNormal::Length(Length::Px(10.0)),
            column: LengthOrNormal::Length(Length::Px(30.0)),
        };
        apply_value(PropertyValue::Gap(g), &mut cv);
        assert_eq!(cv.row_gap, LengthOrNormal::Length(Length::Px(10.0)));
        assert_eq!(cv.column_gap, LengthOrNormal::Length(Length::Px(30.0)));
    }

    #[test]
    fn apply_value_direct_place_content_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::PlaceContent(p)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `AlignContent`/`JustifyContent`
        // longhands before `apply_value` ever sees it) — not a safety
        // net, a canary.
        use crate::property::{ContentAlignmentValue, PlaceContentShorthand};
        let mut cv = SpecifiedValues::initial();
        let p = PlaceContentShorthand {
            align: ContentAlignmentValue::SpaceBetween,
            justify: ContentAlignmentValue::Center,
        };
        apply_value(PropertyValue::PlaceContent(p), &mut cv);
        assert_eq!(cv.align_content, ContentAlignmentValue::SpaceBetween);
        assert_eq!(cv.justify_content, ContentAlignmentValue::Center);
    }

    #[test]
    fn apply_value_direct_grid_row_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::GridRow(shorthand)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `GridRowStart`/`GridRowEnd` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::GridLineShorthand;
        use crate::property::GridLineValue;
        let mut cv = SpecifiedValues::initial();
        let shorthand = GridLineShorthand {
            start: GridLineValue::Line(2),
            end: GridLineValue::Span(3),
        };
        apply_value(PropertyValue::GridRow(shorthand), &mut cv);
        assert_eq!(cv.grid_row_start, GridLineValue::Line(2));
        assert_eq!(cv.grid_row_end, GridLineValue::Span(3));
    }

    #[test]
    fn apply_value_direct_grid_column_shorthand_fall_through() {
        // Sibling of `apply_value_direct_grid_row_shorthand_fall_through`
        // above, for `PropertyValue::GridColumn(shorthand)`.
        use crate::property::GridLineShorthand;
        use crate::property::GridLineValue;
        let mut cv = SpecifiedValues::initial();
        let shorthand = GridLineShorthand {
            start: GridLineValue::Named("content".into()),
            end: GridLineValue::Auto,
        };
        apply_value(PropertyValue::GridColumn(shorthand), &mut cv);
        assert_eq!(cv.grid_column_start, GridLineValue::Named("content".into()));
        assert_eq!(cv.grid_column_end, GridLineValue::Auto);
    }

    #[test]
    fn apply_value_direct_place_items_shorthand_fall_through() {
        // Sibling of `apply_value_direct_place_content_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::PlaceItems(p)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `AlignItems`/`JustifyItems` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{PlaceItemsShorthand, SelfAlignmentValue};
        let mut cv = SpecifiedValues::initial();
        let p = PlaceItemsShorthand {
            align: SelfAlignmentValue::Center,
            justify: SelfAlignmentValue::End,
        };
        apply_value(PropertyValue::PlaceItems(p), &mut cv);
        assert_eq!(cv.align_items, SelfAlignmentValue::Center);
        assert_eq!(cv.justify_items, SelfAlignmentValue::End);
    }

    #[test]
    fn apply_value_direct_place_self_shorthand_fall_through() {
        // Sibling of `apply_value_direct_place_items_shorthand_fall_through`
        // above, for `PropertyValue::PlaceSelf(p)`.
        use crate::property::{AlignSelfValue, PlaceSelfShorthand, SelfAlignmentValue};
        let mut cv = SpecifiedValues::initial();
        let p = PlaceSelfShorthand {
            align: AlignSelfValue::Auto,
            justify: AlignSelfValue::Value(SelfAlignmentValue::Start),
        };
        apply_value(PropertyValue::PlaceSelf(p), &mut cv);
        assert_eq!(cv.align_self, AlignSelfValue::Auto);
        assert_eq!(
            cv.justify_self,
            AlignSelfValue::Value(SelfAlignmentValue::Start)
        );
    }

    #[test]
    fn apply_value_direct_background_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Background(shorthand)` arm
        // is unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 8 `BackgroundColor`/`BackgroundImage`/
        // `BackgroundRepeat`/`BackgroundAttachment`/`BackgroundPosition`/
        // `BackgroundSize`/`BackgroundClip`/`BackgroundOrigin` longhands
        // before `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{
            BackgroundAttachment, BackgroundImage, BackgroundRepeat, BackgroundRepeatKeyword,
            BackgroundShorthand, BackgroundSize, CssPosition, CssPositionOffset, VisualBox,
        };
        let mut cv = SpecifiedValues::initial();
        let shorthand = BackgroundShorthand {
            color: RED,
            image: BackgroundImage::Url("tile.png".to_string()),
            repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Round,
                y: BackgroundRepeatKeyword::Space,
            },
            attachment: BackgroundAttachment::Fixed,
            position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Px(10.0)),
                vertical: CssPositionOffset::End(Length::Px(20.0)),
            },
            size: BackgroundSize::Explicit {
                width: LengthOrAuto::Length(Length::Px(100.0)),
                height: LengthOrAuto::Auto,
            },
            clip: VisualBox::PaddingBox,
            origin: VisualBox::ContentBox,
        };
        apply_value(PropertyValue::Background(shorthand.clone()), &mut cv);
        assert_eq!(cv.background_color, RED);
        assert_eq!(cv.background_image, shorthand.image);
        assert_eq!(cv.background_repeat, shorthand.repeat);
        assert_eq!(cv.background_attachment, shorthand.attachment);
        assert_eq!(cv.background_position, shorthand.position);
        assert_eq!(cv.background_size, shorthand.size);
        assert_eq!(cv.background_clip, shorthand.clip);
        assert_eq!(cv.background_origin, shorthand.origin);
    }

    #[test]
    fn background_shorthand_expands_to_8_longhands_through_real_cascade() {
        // A literal shorthand is expanded before values reach `apply_value`.
        use crate::property::{BackgroundAttachment, BackgroundImage, VisualBox};
        let cv = cascade_doc(
            "",
            "div",
            Some("background: red url(a.png) no-repeat fixed border-box"),
        );
        assert_eq!(cv.background_color, RED);
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("a.png".to_string())
        );
        assert_eq!(cv.background_attachment, BackgroundAttachment::Fixed);
        assert_eq!(cv.background_clip, VisualBox::BorderBox);
        assert_eq!(cv.background_origin, VisualBox::BorderBox);
    }

    #[test]
    fn background_shorthand_comma_separated_multi_layer_declaration_dropped_through_real_cascade() {
        // `property::tests::background_shorthand_rejects_comma_separated_multi_layer`
        // pins this via the `parse_entire` fixture (`Parser::parse_entirely`,
        // semantically equivalent to the real `DeclParser`'s
        // `expect_exhausted` check but not the real pipeline itself). This
        // is the end-to-end sibling, through the real parse -> cascade path
        // (`expand_shorthand_into` never runs since `parse_value` itself
        // returns `None` for the whole declaration): a prior valid winner
        // must survive untouched, not merely fall back to the initial value
        // (which would also happen if the declaration were simply absent).
        let cv = cascade_doc(
            "",
            "div",
            Some("background: red; background: url(a.png) top, url(b.png) bottom"),
        );
        assert_eq!(cv.background_color, RED);
    }

    #[test]
    fn font_shorthand_expands_to_6_longhands_through_real_cascade() {
        // `background_shorthand_expands_to_8_longhands_through_real_cascade`
        // の sibling — literal な `font:` declaration が parse → cascade
        // pipeline を通り 6 longhand に展開されることの pin。
        use crate::property::{FontStyle, FontVariantCaps};
        let cv = cascade_doc(
            "",
            "div",
            Some("font: italic small-caps bold 20px/1.5 serif"),
        );
        assert_eq!(cv.font_style, FontStyle::Italic);
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.line_height, ComputedLineHeight::Number(1.5));
        assert_eq!(cv.font_family[0].to_string(), "serif");
    }

    #[test]
    fn font_shorthand_and_font_style_longhand_interleave_by_source_order() {
        // `background_shorthand_and_background_color_longhand_interleave_by_source_order`
        // の sibling — shorthand と longhand の競合は source order で決まる。
        use crate::property::FontStyle;
        let shorthand_first = cascade_doc(
            "",
            "div",
            Some("font: italic 20px serif; font-style: normal"),
        );
        assert_eq!(shorthand_first.font_style, FontStyle::Normal);

        let longhand_first = cascade_doc("", "div", Some("font-style: italic; font: 20px serif"));
        assert_eq!(longhand_first.font_style, FontStyle::Normal);
    }

    #[test]
    fn font_shorthand_relative_size_and_weight_resolve_against_parent() {
        // `div` は root (16px / 400) の子 — `larger` / `bolder` は親基準で
        // 解決される (longhand arm と同じ roll)。
        let cv = cascade_doc("", "div", Some("font: bolder larger serif"));
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, ComputedLength(19.2));
    }

    #[test]
    fn apply_value_direct_font_shorthand_fall_through() {
        // `apply_value_direct_margin_shorthand_fall_through` の sibling —
        // cascade 経路では unreachable (`expand_shorthand_into` が展開済み)
        // の canary。relative 成分 (`bolder` / `larger`) は longhand arm と
        // 同じく parent seed (ここでは initial: 400 / 16px) 基準で解決される。
        use crate::Atom;
        use crate::property::{
            FontShorthand, FontShorthandSize, FontStyle, FontVariantCaps, FontWeightValue,
            LineHeight, RelativeFontSize,
        };
        use std::sync::Arc;
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Font(FontShorthand {
                style: FontStyle::Italic,
                variant: FontVariantCaps::SmallCaps,
                weight: FontWeightValue::Bolder,
                size: FontShorthandSize::Relative(RelativeFontSize::Larger),
                line_height: LineHeight::Number(1.5),
                family: Arc::new(vec![Atom::from("serif")]),
            }),
            &mut cv,
        );
        assert_eq!(cv.font_style, FontStyle::Italic);
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, Length::Px(19.2));
        assert_eq!(cv.line_height, LineHeight::Number(1.5));
        assert_eq!(cv.font_family[0].to_string(), "serif");
        // Absolute size takes the plain `FontSize` longhand arm (same
        // delegation shape as the `Relative` case above).
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Font(FontShorthand {
                style: FontStyle::Normal,
                variant: FontVariantCaps::Normal,
                weight: FontWeightValue::Absolute(400.0),
                size: FontShorthandSize::Absolute(Length::Px(12.0)),
                line_height: LineHeight::Normal,
                family: Arc::new(vec![Atom::from("serif")]),
            }),
            &mut cv,
        );
        assert_eq!(cv.font_size, Length::Px(12.0));
    }

    #[test]
    fn background_shorthand_and_background_color_longhand_interleave_by_source_order() {
        // Mirror of `var_in_margin_shorthand_preserves_later_longhand_cascade`'s
        // sibling check (`margin-top: 10px; margin: 0px` → all sides 0): the
        // `expand_shorthand_into` doc's entire rationale is that source order,
        // not variant order, decides the winner once a shorthand and a
        // conflicting longhand both target the same field. `background` is
        // the first shorthand whose fan-out targets 8 *pre-existing*
        // longhands rather than a fresh `Sides<T>`-style bundle, so this
        // pins that the same mechanism holds for it in both directions.
        let shorthand_first =
            cascade_doc("", "div", Some("background: red; background-color: blue"));
        assert_eq!(shorthand_first.background_color, BLUE);

        let longhand_first =
            cascade_doc("", "div", Some("background-color: blue; background: red"));
        assert_eq!(longhand_first.background_color, RED);
    }

    #[test]
    fn text_decoration_shorthand_expands_to_line_style_and_color() {
        // `text-decoration: underline wavy red` — all 3 longhand winners
        // reach the same node through real parse + cascade.
        let cv = cascade_doc("", "div", Some("text-decoration: underline wavy red"));
        assert_eq!(cv.text_decoration_line, TextDecorationLine::UNDERLINE);
        assert_eq!(cv.text_decoration_style, TextDecorationStyle::Wavy);
        assert_eq!(cv.text_decoration_color, TextDecorationColor::Resolved(RED));
    }

    #[test]
    fn text_decoration_inset_resolves_and_keeps_auto_distinct() {
        let cv = cascade_doc(
            "",
            "div",
            Some("font-size: 20px; text-decoration-inset: 1em 2px"),
        );
        assert_eq!(
            cv.text_decoration_inset,
            ComputedTextDecorationInset::Lengths {
                start: ComputedLength(20.0),
                end: ComputedLength(2.0),
            }
        );

        let auto = cascade_doc("", "div", Some("text-decoration-inset: auto"));
        assert_eq!(
            auto.text_decoration_inset,
            ComputedTextDecorationInset::Auto
        );
    }

    #[test]
    fn text_decoration_inset_is_non_inherited() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("text-decoration-inset: 3px 4px"));
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.computed[parent].text_decoration_inset,
            ComputedTextDecorationInset::Lengths {
                start: ComputedLength(3.0),
                end: ComputedLength(4.0),
            }
        );
        assert_eq!(
            result.computed[child].text_decoration_inset,
            ComputedValues::initial().text_decoration_inset
        );
    }

    #[test]
    fn text_decoration_shorthand_resets_earlier_longhand_declarations() {
        // Shorthand-resets-omitted-longhands (CSS Cascading L4 §3 "exactly
        // as if expanded in place"): `text-decoration: underline` omits the
        // style/color components, but `parse_text_decoration_shorthand`
        // fills them with their *own* initial values rather than leaving
        // them unset — so a later bare `text-decoration: underline` still
        // resets an earlier explicit `text-decoration-style: wavy` back to
        // `solid` through ordinary "later declaration in the same block
        // wins" cascade order (CSS Cascading L4 §6.1 "Order of Appearance").
        // This is the test that actually discriminates a spec-correct
        // expansion from one that merely "leaves the others alone" — see
        // `crate::rule::tests::text_decoration_shorthand_always_overwrites_all_four_longhand` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — rustdoc-blind, confirmed via わざと壊して確かめる
        // for the declaration-list-shape version of the same fact.
        let cv = cascade_doc(
            "",
            "div",
            Some("text-decoration-style: wavy; text-decoration: underline"),
        );
        assert_eq!(cv.text_decoration_line, TextDecorationLine::UNDERLINE);
        assert_eq!(
            cv.text_decoration_style,
            TextDecorationStyle::Solid,
            "the later `text-decoration` shorthand must reset style back to \
             its own initial value, not leave the earlier `wavy` in place"
        );
        assert_eq!(cv.text_decoration_color, TextDecorationColor::CurrentColor);
    }

    #[test]
    fn text_decoration_shorthand_then_longhand_later_longhand_wins() {
        // Mirror of `border_shorthand_then_longhand_later_longhand_wins`:
        // `text-decoration: underline wavy; text-decoration-style: dotted;`
        // → style ends up `dotted` (later longhand wins), line stays
        // `underline` (untouched by the longhand declaration).
        let cv = cascade_doc(
            "",
            "div",
            Some("text-decoration: underline wavy; text-decoration-style: dotted"),
        );
        assert_eq!(cv.text_decoration_line, TextDecorationLine::UNDERLINE);
        assert_eq!(cv.text_decoration_style, TextDecorationStyle::Dotted);
    }

    #[test]
    fn text_decoration_non_inherited_child_starts_from_initial() {
        // CSS Text Decoration Module Level 3 §2.1-§2.3 — all 3 propdefs are
        // "Inherited: no". sibling: border / margin / padding non-inherited
        // test pattern.
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("text-decoration: underline wavy red"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].text_decoration_line,
            TextDecorationLine::UNDERLINE
        );
        assert_eq!(
            r.computed[span].text_decoration_line,
            TextDecorationLine::NONE,
            "text-decoration-line must not inherit from parent"
        );
        assert_eq!(
            r.computed[span].text_decoration_style,
            TextDecorationStyle::Solid
        );
        assert_eq!(
            r.computed[span].text_decoration_color,
            TextDecorationColor::CurrentColor
        );
    }

    #[test]
    fn width_length_end_to_end() {
        // Verification #7: `div { width: 100px }` が `ComputedValues.width` に
        // Length(Px(100)) として届く。parser → PropertyValue::Width → apply_value
        // → ComputedValues の end-to-end 疎通 smoke (sibling padding/margin と
        // 同 pattern)。
        let cv = cascade_doc("", "div", Some("width: 100px"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn width_auto_end_to_end() {
        // `width: auto` は cascade winner として apply_value で `Auto` に固定される。
        // `inherit_from` initial も Auto なので identity になるが、cascade path が
        // 実際に通っていることを check (silent no-op regression 検知)。
        let cv = cascade_doc("", "div", Some("width: auto"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn width_default_is_initial_auto() {
        // 未指定時は `ComputedValues::initial()` の Auto を維持 (spec §3.1.1
        // "Initial: auto"、non-inherited なので parent も影響しない)。
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn width_child_does_not_inherit_from_parent() {
        // Verification #8: parent (div) が width: 100px を持っていても child
        // (span、指定 無し) は initial (Auto) を保持する。non-inherited property
        // の end-to-end check (sibling `inherit_from_leaves_*_at_initial` computed
        // 側 test の cascade path 版)。
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div { width: 100px }");
        let parent = doc.push_element(0, "div", None);
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).unwrap();
        // parent (div) は width: 100px を受け取る
        assert_eq!(
            result.computed[parent].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        // child (span) は non-inherited のため initial (Auto) を保持
        assert_eq!(
            result.computed[child].width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn min_width_length_end_to_end() {
        // CSS Sizing 3 §4: `div { min-width: 100px }` が
        // `ComputedValues.min_width` に Px(100) として届く。sibling
        // `width_length_end_to_end` と同 pattern (parse_min_size →
        // PropertyValue::MinWidth → apply_value → finalize の end-to-end
        // smoke)。
        let cv = cascade_doc("", "div", Some("min-width: 100px"));
        assert_eq!(cv.min_width, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn min_height_auto_and_negative_reject() {
        // CSS Sizing 3 §4 initial `auto` の identity round-trip + `[0,∞]`
        // 違反の drop pin。負値は parse 段で declaration drop するため
        // initial (Auto) のまま残る。
        let cv = cascade_doc("", "div", Some("min-height: auto"));
        assert_eq!(cv.min_height, ComputedLengthPercentageOrAuto::Auto);
        let cv_neg = cascade_doc("", "div", Some("min-height: -10px"));
        assert_eq!(cv_neg.min_height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn min_max_child_does_not_inherit_from_parent() {
        // CSS Sizing 3 §4/§5: min/max は **non-inherited**。parent が
        // min-width / max-width を持っていても child は initial を保持する
        // (sibling `width_child_does_not_inherit_from_parent` と同 pattern)。
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div { min-width: 100px; max-width: 200px }");
        let parent = doc.push_element(0, "div", None);
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).unwrap();
        assert_eq!(
            result.computed[parent].min_width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(
            result.computed[parent].max_width,
            ComputedLengthPercentageOrAuto::Px(200.0)
        );
        assert_eq!(
            result.computed[child].min_width,
            ComputedLengthPercentageOrAuto::Auto
        );
        assert_eq!(
            result.computed[child].max_width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn max_width_none_maps_to_auto() {
        // CSS Sizing 3 §5 initial `none` → computed Auto placeholder の連鎖
        // check (specified `parse_max_size` の `none` 分岐 + bridge の
        // `Dimension::auto()` 委譲の上流側)。
        let cv = cascade_doc("", "div", Some("max-width: none"));
        assert_eq!(cv.max_width, ComputedLengthPercentageOrAuto::Auto);
        let cv_px = cascade_doc("", "div", Some("max-height: 50%"));
        assert_eq!(
            cv_px.max_height,
            ComputedLengthPercentageOrAuto::Percent(50.0)
        );
    }

    fn cascade_with_post_parse_injection(
        css: &str,
        idx: usize,
        injected: PropertyValue,
    ) -> ComputedValues {
        let mut doc = TestDoc::new();
        let e = doc.push_element(0, "div", None);
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::Author);
        tree.style_rules[0].declarations[idx].value = injected;
        let result = cascade(&doc, &tree).expect("cascade Ok");
        result.computed[e].clone()
    }

    fn distinct_margin_sides() -> Sides<LengthOrAuto> {
        Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        }
    }

    #[test]
    fn post_parse_margin_shorthand_before_longhand_lets_longhand_win() {
        // 注入後の declaration 列 (= `margin: 1px 2px 3px 4px; margin-top: 10px`):
        //   `[0]` Margin(1,2,3,4)   ← 注入
        //   `[1]` MarginTop(10px)
        // spec §3 + §6.1 → top=10 (後方 longhand)、right/bottom/left=2/3/4。
        //
        // これが nqkj の報告する spec 違反方向。展開しない実装では
        // `PropertyKey::Margin` が `MarginTop` より後に適用されるので
        // 全 side が 1/2/3/4 になり top=10 が破壊される。
        let cv = cascade_with_post_parse_injection(
            "div { margin-left: 99px; margin-top: 10px }",
            0,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(10.0),
            "後方 longhand が order of appearance で勝つこと (§6.1)"
        );
        // 残り 3 side は shorthand 由来の per-side 値。全て assert するのは
        // 「shorthand を単に落とす」実装 (top=10 だが他が initial 0 になる) と
        // 「top arm だけ展開する」実装を弾くため。
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn post_parse_margin_shorthand_after_longhand_lets_shorthand_win() {
        // 鏡像方向 (`margin-top: 10px; margin: 1px 2px 3px 4px`):
        //   `[0]` MarginTop(10px)
        //   `[1]` Margin(1,2,3,4)   ← 注入
        // spec §6.1 → 全 side が shorthand 由来 = 1/2/3/4。
        //
        // 本方向は展開しない実装でも偶然一致するが、fix が「shorthand を
        // 常に負けさせる」誤った非対称化になっていないことを check する。
        let cv = cascade_with_post_parse_injection(
            "div { margin-top: 10px; margin-left: 99px }",
            1,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(1.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn post_parse_padding_shorthand_before_longhand_lets_longhand_win() {
        // margin と同じ形を padding family でも check (展開 arm が family ごとに
        // 独立に書かれているため)。1/2/3/4px の意図は `distinct_margin_sides`
        // doc と同じ。
        let cv = cascade_with_post_parse_injection(
            "div { padding-left: 99px; padding-top: 10px }",
            0,
            PropertyValue::Padding(Sides {
                top: Length::Px(1.0),
                right: Length::Px(2.0),
                bottom: Length::Px(3.0),
                left: Length::Px(4.0),
            }),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(2.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(3.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(4.0));
    }

    #[test]
    fn post_parse_border_shorthand_before_longhand_lets_longhand_win() {
        // border は 4 side × 3 sub-property = 12 longhand に展開される。
        // width / style を per-side で全て違う値にして、12 arm が sink 経由でも
        // 落ちていないことを check する (style は computed width の gating にも
        // 効くので `None` を混ぜない — CSS Backgrounds 3 §3.3)。width の
        // 1/2/3/4px の意図は `distinct_margin_sides` doc と同じ。
        let cv = cascade_with_post_parse_injection(
            "div { border-left-width: 99px; border-top-width: 10px }",
            0,
            PropertyValue::Border(Sides {
                top: Border {
                    width: Length::Px(1.0),
                    style: BorderStyle::Solid,
                    color: BorderColor::Resolved(RED),
                },
                right: Border {
                    width: Length::Px(2.0),
                    style: BorderStyle::Dashed,
                    color: BorderColor::Resolved(BLUE),
                },
                bottom: Border {
                    width: Length::Px(3.0),
                    style: BorderStyle::Dotted,
                    color: BorderColor::CurrentColor,
                },
                left: Border {
                    width: Length::Px(4.0),
                    style: BorderStyle::Double,
                    color: BorderColor::Resolved(RED),
                },
            }),
        );
        // top.width だけ後方 longhand が勝つ。
        assert_eq!(cv.border.top.width, ComputedLength(10.0));
        assert_eq!(cv.border.right.width, ComputedLength(2.0));
        assert_eq!(cv.border.bottom.width, ComputedLength(3.0));
        assert_eq!(cv.border.left.width, ComputedLength(4.0));
        // style / color は shorthand 由来のまま per-side に残る。
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
        assert_eq!(cv.border.right.style, BorderStyle::Dashed);
        assert_eq!(cv.border.bottom.style, BorderStyle::Dotted);
        assert_eq!(cv.border.left.style, BorderStyle::Double);
        assert_eq!(cv.border.top.color, BorderColor::Resolved(RED));
        assert_eq!(cv.border.right.color, BorderColor::Resolved(BLUE));
        assert_eq!(cv.border.bottom.color, BorderColor::CurrentColor);
        assert_eq!(cv.border.left.color, BorderColor::Resolved(RED));
    }

    #[test]
    fn post_parse_shorthand_injection_propagates_important() {
        // CSS Cascading Level 4 §3 makes a shorthand's `!important`
        // flag apply to all expanded longhands.
        let cv = cascade_with_post_parse_injection(
            "div { margin-left: 99px !important; margin-top: 10px }",
            0,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(1.0),
            "important shorthand 由来の MarginTop が normal longhand に勝つこと"
        );
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn post_parse_important_longhand_survives_later_normal_shorthand() {
        // 注入後の declaration 列
        // (= `margin-top: 10px !important; margin: 1px 2px 3px 4px`):
        //   `[0]` MarginTop(10px) !important
        //   `[1]` Margin(1,2,3,4)  normal   ← 注入 (`important` は false のまま)
        //
        // CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
        // の cascade sort は Origin and Importance を Order of Appearance
        // **より上位**に置く。したがって後方の normal shorthand は前方の
        // important longhand に勝てない → top=10。残り 3 side は shorthand 由来
        // = 2/3/4。
        //
        // **`!important` 方向で「展開しない実装」と区別できるのは本 test 群では
        // これだけである** — 展開しないと `PropertyKey::Margin` slot が独立
        // winner になり、discriminant 順 (`Margin` > `MarginTop`) で atomic
        // 上書きして top=1 になる。同じ importance を両者に持たせた
        // `post_parse_shorthand_injection_propagates_important` では
        // 偶然一致してしまい区別できない。
        //
        // (鏡像入力 `margin: 1,2,3,4; margin-top: 10px !important` や
        // padding / border family の同型入力も同様に区別する — 本 test が
        // 唯一というわけではない。coverage 追加は歓迎。)
        let cv = cascade_with_post_parse_injection(
            "div { margin-top: 10px !important; margin-left: 99px }",
            1,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(10.0),
            "important longhand が後方の normal shorthand 由来 longhand に勝つこと \
             (§6.1 Origin and Importance)"
        );
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn float_wired_through_cascade_from_inline_style() {
        // <p style="float: left"> → ComputedValues.float に FloatValue::Left
        // が届く。parser → PropertyValue::Float → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。sibling (z-index) の
        // wire-through pattern を踏襲。
        use crate::property::FloatValue;
        let cv = cascade_doc("", "p", Some("float: left"));
        assert_eq!(cv.float, FloatValue::Left);
    }

    #[test]
    fn float_non_inherited_child_starts_from_initial() {
        // CSS2 §9.5.1 propdef: "Inherited: no". sibling:
        // `z_index_non_inherited_child_starts_from_initial` と同じ pattern。
        use crate::property::FloatValue;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("float: right"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].float, FloatValue::Right);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].float,
            FloatValue::None,
            "float must not inherit from parent (CSS2 §9.5.1 Inherited: no)"
        );
    }

    #[test]
    fn clear_wired_through_cascade_from_inline_style() {
        // <p style="clear: both"> → ComputedValues.clear に ClearValue::Both
        // が届く。parser → PropertyValue::Clear → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。sibling (z-index) の
        // wire-through pattern を踏襲。
        use crate::property::ClearValue;
        let cv = cascade_doc("", "p", Some("clear: both"));
        assert_eq!(cv.clear, ClearValue::Both);
    }

    #[test]
    fn clear_non_inherited_child_starts_from_initial() {
        // CSS2 §9.5.2 propdef: "Inherited: no". sibling:
        // `z_index_non_inherited_child_starts_from_initial` と同じ pattern。
        use crate::property::ClearValue;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("clear: left"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].clear, ClearValue::Left);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].clear,
            ClearValue::None,
            "clear must not inherit from parent (CSS2 §9.5.2 Inherited: no)"
        );
    }

    #[test]
    fn inherited_border_radius_handles_percent() {
        assert_eq!(
            inherited_border_radius(ComputedLengthPercentage::Percent(25.0)),
            Length::Percent(25.0)
        );
        assert_eq!(
            inherited_border_radius(ComputedLengthPercentage::Px(4.0)),
            Length::Px(4.0)
        );
    }
}
