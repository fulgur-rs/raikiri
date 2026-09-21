use std::collections::HashMap;
use std::sync::Arc;

use crate::PseudoElem;
use crate::computed::{
    ComputedValues, CustomPropertyEnvironment, RunningTemplate, empty_custom_properties,
};
use crate::property::{
    BorderRadius, FontWeightValue, GridAutoFlowValue, GridLineValue, GridTemplateAreasValue,
    Length, LengthOrAuto, LengthOrNormal, PositionValue, PropertyValue, RelativeFontSize, Sides,
    WritingMode, initial_grid_auto_track_list, resolve_text_align_match_parent,
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
/// 以前は `HashMap` iteration 順 (per-process random seed) だった。既存
/// property は key と [`SpecifiedValues`] の field が 1:1 disjoint なので、
/// 決定的になったこと自体に観測可能な差は無い。
///
/// ## shorthand key はここに届かない (なぜ順序に賭けてはいけないか)
///
/// shorthand key (`PropertyKey::{Padding, Margin, Border}`) は宣言順では対応する
/// longhand より**後**に来るので、**もし**届いたら必ず後勝ちする。それが spec と
/// 食い違うかどうかは declaration の並び順に依存する:
///
/// ```text
/// margin: 0px; margin-top: 10px  → spec top=10 / 順序任せなら top=0   ← 食い違う
/// margin-top: 10px; margin: 0px  → spec top=0  / 順序任せなら top=0   ← 一致
/// ```
///
/// spec は「longhand が勝つ」とは言っていない。CSS Cascading L4 §3
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> が shorthand を
/// "exactly as if expanded in place" と定義し、勝者は §6.1
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の "the last declaration
/// in document order wins" で決まるだけである。
///
/// **したがって `PropertyKey` の variant 順を入れ替える fix は誤り** — 上の
/// 2 例は鏡像なので、shorthand を longhand の前に動かすと今度は 2 番目の case
/// が spec 違反になる。正しい fix は「shorthand を cascade 段に**到達させない**」
/// 方向にしかなく、実装はそちらを採っている:
///
/// - **inline style** — [`collect_cascaded`] が
///   [`crate::rule::parse_declaration_block`] を通すので parse 出口で展開済み。
/// - **stylesheet rule** — [`collect_cascaded`] が candidate に積む直前に
///   [`crate::rule::expand_shorthand_into`] を通す。
///   parse 出口の展開だけでは post-parse mutation 経路を守れないため
///   (この経路は crate 内限定 — 根拠は
///   [`crate::rule::expand_shorthand_into`] doc が canonical)。**shorthand が
///   到達したら既に bug** なので削除可能な dead defensive code ではない。
///
/// 展開後は同一 key の longhand が複数 candidate になるが、[`beats`] の `>=`
/// が「同 rank/spec/order なら後方勝ち」を与えるので §6.1 の order of appearance
/// がそのまま成立する。
///
/// 「展開 arm の書き忘れ」形の壊れ方は **compile-time に強制されている**
/// — [`crate::rule::expand_shorthand_into`] の match は
/// exhaustive で、`PropertyValue` に variant を足すと同関数に arm を書くまで
/// compile error になる。arm list は 1 関数に集約されているので上の 2 経路が
/// 同時に保証を得る。**強制されるのは arm を書くことだけ**で、残る範囲は同関数
/// doc の「この guard が守らない範囲」節。defense-in-depth の runtime
/// guard は [`crate::rule`] の `declaration_block_never_emits_shorthand_keys`
/// (parse 出口) と、本 module の `post_parse_*` test 群
/// (6 本、`RuleTree` post-parse mutation 経路 — うち展開の有無を実際に区別する
/// のは 4 本。残り 2 本は over-correction 用の弱い guard で、各 test の comment
/// にその旨を開示してある)。
///
/// # `candidates[winner.idx]` の unchecked index について
///
/// `winner.idx` は直前の [`pick_winners`] が **同じ `candidates`** に対して
/// 作ったものなので in-bounds。fill と drain が本関数 body 内で隣接しており、
/// 間に `candidates` を差し替える経路が無いことが根拠。
///
/// **この bare index は防御機構ではない。** slot が前 node から漏れた場合、
/// 漏れた `idx` は次 node の candidate list に対しても通常 in-bounds なので
/// (candidate list はたいてい漏れた index より長い)、panic せず**別 node の
/// declaration を静かに適用**してしまう。実際
/// `winner_does_not_leak_into_next_sibling` が捕まえるのも panic ではなく
/// in-bounds の二重適用である。leak に対する実効的な net は [`pick_winners`]
/// 冒頭の debug_assert (**release build では消える**) と同 test の 2 つ。
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
fn apply_winners(
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
/// **本関数自体には runtime guard を追加しない** (「非有限 / 範囲外 f32 の
/// guard は sink 境界に置く、resolve 層には置かない」という既存方針を
/// 踏襲)。上記の非対称処理は
/// `resolve_relative_weight_non_finite_inherited_is_asymmetric` test で
/// 現状の挙動として check 済み。値が実際に `parley::FontWeight::new` へ渡る
/// sink 側の guard は `crates/raikiri-dom/src/layout.rs` の
/// `sanitize_font_weight` (`preshape_text` 内、site 6) にある —
/// `resolve_relative_weight` が何を返しても最終的に `[1, 1000]` の有限値に
/// 収める。
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
/// 実際に過去に起きた regression (`@page { font-weight: bolder }` が
/// `FontWeightValue::Bolder` のまま park していた) がまさにこの形
/// だった。exhaustive match なら variant 追加が本関数と [`apply_value`] の
/// **両方**で compile error になり、2 経路を数え上げることが強制される。
///
/// # この guard が守らない範囲 (明示)
///
/// 本 match が compile error で捕まえるのは **`PropertyValue` の variant 追加**
/// だけである。以下 2 つは捕まらない:
///
/// 1. 既存 variant の **payload** に継承元依存が入る場合 — pass-through arm は
///    payload を `_` で捨てるので、`TextAlign` に `MatchParent` (CSS Text 3) が
///    増えても `PropertyValue::TextAlign(_)` を素通りする。これは `bolder` /
///    `lighter` と**同型**の解決を要するので、実装時は本関数の arm で payload を
///    destructure して guard を payload 層に降ろすこと (`FontSize` /
///    `TextAlign` は既にそれを済ませてある — 前者は payload を destructure
///    して `resolve_font_size` に、
///    後者は [`crate::property::resolve_text_align_match_parent`] に渡している)。
///    **page 経路の、かつ payload 型が
///    `Length` / `LengthOrAuto` / `LineHeight` / `FontWeightValue` /
///    `TextAlign` の 5 つに限れば**、この形の漏れは `page::tests` の
///    `specified_layer_residue` が網羅 match しているので test compile 段で
///    捕まる。それ以外 (`BorderStyle` / `BorderColor`
///    / `DisplayValue` / `PositionValue` / `BoxSizing` / `ContentComponent`
///    / `OverflowValue` / `TextDecorationLine` / `TextDecorationStyle` /
///    `TextDecorationColor` / `VerticalAlign` / `FontStyle` / `Visibility` /
///    `ZIndexValue` / `WordBreak` / `OverflowWrap` / `BreakBetween` /
///    `BreakInside` / `WhiteSpace` / `Hyphens` / `FontVariantCaps`)
///    は同検出器も `_` で捨てており、`Border` struct の field 追加も
///    field access で読んでいるため捕まらない。compile error になるのも
///    test target であって本関数ではない。
/// 2. **本関数を呼ばない新しい entry point** — 過去の regression はこの形
///    だった (`cascade_page` が `apply_value` を通らなかった)。CSS Paged Media 3
///    §6 の margin-box cascade は page context を継承元とする第 3 の経路になる。
///    exhaustive match は「経路の数え上げ」を強制しない。
///
///    この穴を **型で狭めた** (完全には塞いでいない)
///    — 本関数の戻り値は生の [`PropertyValue`] ではなく
///    [`ResolvedAgainstInherited`]。その型の doc「narrowed, not closed」節が
///    canonical な記述 (何を防ぎ、何を防がないか、残余は別途 margin-box
///    cascade 側の課題として残ること) を持つので、ここでは
///    繰り返さない。
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
/// 公開契約は
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// が canonical。以前あった「public な結果に残る例外はこれ 1 つ」は
/// 解消され、`page::tests` の
/// `page_declarations_carry_no_specified_layer_residue` (旧
/// `page_declarations_carry_exactly_one_specified_layer_residue`) が
/// check する。
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
        PropertyValue::TextAlign(t) => PropertyValue::TextAlign(resolve_text_align_match_parent(
            t,
            inherited.text_align,
            inherited.direction,
        )),
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
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
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
/// # test 用の裏口 (`Self::for_test`)
///
/// `page::tests` には phase 3 を意図的に phase 2 抜きで直接駆動する既存 test
/// 群がある (`phase_3_variant_classification_matches_the_documented_counts` /
/// `absolutize_in_page_context_shorthand_fall_throughs` /
/// `absolutize_in_page_context_font_size_relative_safety_net` —
/// いずれも「structurally unreachable だが `pub(crate)` 関数は直接駆動できる」
/// という既存の defense-in-depth 方針の precedent)。これらが本型導入後も raw payload を直接検査できるよう、
/// `#[cfg(test)]` 限定の直接 constructor を用意する。production build には
/// 存在しないので、上記の「他 module は本関数を呼ぶ以外に値を作れない」
/// production guarantee は弱めない。
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

    /// **test 専用の直接 constructor。** 上記型 doc「test 用の裏口」参照 —
    /// production では存在しない (`#[cfg(test)]`)。**この `#[cfg(test)]` を
    /// 外したくなったら、それは「この型が防ぐはずの bypass」を作ろうとして
    /// いる signal である** — 代わりに [`resolve_against_inherited`] を経由
    /// すること。
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
        // ⚠️ **これは "safety" net ではない**。
        // element cascade 経由では到達不能 — `collect_cascaded` が candidate を
        // 積む前に、stylesheet rule は `expand_shorthand_into` で、inline style は
        // `parse_declaration_block` (内部で同関数を呼ぶ) で展開されるため。
        // 残る到達手段は crate 内から `apply_value` を直接呼ぶことだけである。
        // 仮に到達すると、`PropertyKey` 宣言順では `Margin` が `MarginTop` 等より
        // **後**に適用される。したがってこの atomic 上書きは **longhand winner を
        // 必ず破壊する** — `margin: 0; margin-top: 10px` が top=0 になり
        // CSS Cascading L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand> と
        // 食い違う。net は degraded ではなく **deterministic に spec 違反**。
        // 到達した時点で既に bug であり、本 arm はそれを穏当に見せない。
        //
        // それでも `unreachable!` を採らないのは panic surface を作らない
        // 方針による。cascade 経路で unreachable なのは
        // `crate::rule::expand_shorthand_into` の call site 1 / 2 (parse 出口と
        // element cascade 入口) が担保しており、その担保のうち「展開 arm の
        // 書き忘れ」は同関数の exhaustive match で compile-time に排除されている
        // (残る範囲は同関数 doc の
        // 「この guard が守らない範囲」節)。振る舞い自体は
        // `apply_value_direct_margin_shorthand_fall_through` test が直接叩いて pin。
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
        // staging field yet (parsing only). The keyword-only ones
        // (`LineBreak`...`UnicodeBidi`, `TextDecorationSkipInk`...
        // `TextUnderlinePosition`) carry no length; `TextDecorationThickness`
        // / `TextDecorationInset` carry `<length-percentage>` but are still
        // staging-less — their absolutization lives only on the `@page`
        // path (`crate::page::absolutize_in_page_context`'s arms), same
        // split as the phase-2/phase-3 division above.
        PropertyValue::LineBreak(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextDecorationInset(_)
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
