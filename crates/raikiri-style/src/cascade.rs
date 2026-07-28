//! CSS cascade + inheritance walk (M1.4)。
//!
//! 2 phase:
//! 1. per-node "cascaded values" 決定 — matching rule + inline style の候補集合から
//!    specificity + !important + source order で winner を選択
//! 2. inheritance walk — top-down DFS で親の computed value を継承 + 自 node の
//!    cascaded value で override
//!
//! # inheritance walk 内部の 3 phase (bd decision raikiri-spike-082k)
//!
//! 上記 phase 2 の per-node 処理は、さらに 3 段に分かれる:
//!
//! 1. **winner の staging** — 親の [`ComputedValues`] から
//!    [`SpecifiedValues`] を seed し、その node の全 winner を `apply_value` で
//!    適用する。この段では length は specified 表現のまま。
//! 2. **font-size の絶対化** — **親の** computed font-size 基準。
//! 3. **残り全 length の絶対化** — **自 node の** computed font-size 基準。
//!
//! 2 / 3 は [`SpecifiedValues::finalize`] に閉じている。分離が必要な理由は
//! [`crate::specified`] の module doc を参照 (`padding: 2em` の基準となる
//! `font-size` はその node の**全** winner を適用し終えるまで確定しないため、
//! winner 適用の途中で絶対化することはできない)。

use std::collections::HashMap;

use cssparser::{Parser, ParserInput};
use selectors::parser::{Selector, SelectorList};

use crate::RaikiriSelectorImpl;
use crate::computed::{ComputedValues, RunningTemplate};
use crate::error::CascadeError;
use crate::property::{FontWeightValue, Length, PositionValue, PropertyValue};
use crate::resolve::{ComputedLength, ResolveContext};
use crate::rule::{expand_shorthand_into, parse_declaration_block};
use crate::ruletree::Origin;
use crate::ruletree::RuleTree;
use crate::specified::SpecifiedValues;
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

/// Cascade 結果。
///
/// M1.4 では `computed` のみ populate。M5 static-side (raikiri-spike-m5.1 /
/// m5.3 / m5.4) は per-node ComputedValues 内で content / string_set /
/// running_templates を保持する canonical taxonomy に落ち着き
/// (bd raikiri-spike-376 amended)、CascadeResult-level の `gcpm_directives` /
/// `running_templates` は下流 (raikiri-dom) で per-document に concatenate される
/// 責務に移った。`#[non_exhaustive]` は将来 field 追加のために維持。
#[derive(Debug)]
#[non_exhaustive]
pub struct CascadeResult {
    /// Per-node computed values (NodeId.0 as usize で index)。
    /// Element / Text / Document 全 kind に populate、範囲外は panic (caller 責任)。
    pub computed: Vec<ComputedValues>,
}

/// DOM + RuleTree から per-node ComputedValues を produce。
///
/// M1.4 では常に `Ok` を返す (invalid CSS は既に build_rule_tree 段で silently
/// drop されており、cascade は construct され得ない)。`Result` signature は
/// 将来 fail-hard mode 用に維持。
///
/// # Example
///
/// ```ignore
/// // ignore: raikiri-dom crate は raikiri-style の doc-test から使えないため
/// // (crate cycle 回避)、実 code は integration test で確認。ここは shape のみ。
/// use raikiri_style::{build_rule_tree, cascade, ComputedValues};
///
/// # fn demo<D: raikiri_style::StyleDom>(dom: &D) {
/// let rule_tree = build_rule_tree(dom);
/// let result = cascade(dom, &rule_tree).expect("m1.4 では常に Ok");
/// let root_style: &ComputedValues = &result.computed[0];
/// # }
/// ```
pub fn cascade<D: StyleDom>(dom: &D, rule_tree: &RuleTree) -> Result<CascadeResult, CascadeError> {
    let mut cascaded: HashMap<StyleNodeId, Vec<CascadedDecl>> = HashMap::new();

    // Phase 1: per-node cascaded values を収集
    collect_cascaded(dom, dom.root_id(), rule_tree, &mut cascaded);

    // Phase 2: inheritance walk。
    //
    // raikiri-spike-37c roborev job 293 M1 finding: computed を Dom::node_count()
    // で pre-allocate する。resolve_inheritance の DFS は root reachable な node
    // のみを訪問するため、detached / unreachable node (foster-parenting transient、
    // strip 後の孤児 stub 等) には entry を作らない。しかし raikiri-spike-m1.23
    // contract `computed.len() == document.node_count()` は arena 全体を要求する
    // (`raikiri-dom::layout::preshape_text` / `raikiri-paint::text::draw_text_node`
    // が node_id で `computed[idx]` に直接 index する)。事前に initial() で埋めて
    // おき、DFS で visited slot を上書きする実装。
    let mut computed: Vec<ComputedValues> = vec![ComputedValues::initial(); dom.node_count()];
    resolve_inheritance(
        dom,
        dom.root_id(),
        &ComputedValues::initial(),
        &cascaded,
        &mut computed,
    );

    Ok(CascadeResult { computed })
}

/// selectors 由来の 32-bit specificity。u32 で完全順序比較。
type Specificity = u32;

/// inline style の specificity — spec §6.3 で `(1, 0, 0, 0)` に相当。
/// selectors crate は 32-bit packed で `id << 20 | class << 10 | element` を使うので、
/// inline の 1,0,0,0 相当は 1 << 30 とみなす (どの selector 由来 spec より大)。
const INLINE_SPECIFICITY: Specificity = 1 << 30;
/// inline style の source_order — 全 stylesheet rule より後 (最終出現扱い)。
const INLINE_SOURCE_ORDER: u32 = u32::MAX;

/// 1 candidate declaration = `(value, important, origin, specificity, source_order)`。
/// `collect_cascaded` が populate、`pick_winners` が rank 化して winner を選ぶ
/// (raikiri-spike-m1.22 で `Origin` を追加、clippy::type_complexity 回避のため alias 化)。
type CascadedDecl = (PropertyValue, bool, Origin, Specificity, u32);

/// [`pick_winners`] の scratch slot — 1 property key の暫定勝者。
///
/// [`idx`](Self::idx) が [`PropertyValue`] 本体ではなく **index** なのが要点
/// (bd raikiri-spike-8kn8):
///
/// - slot が `Copy` になり `Drop` を持たないので、slot の reset が
///   [`Option::take`] だけで済む (buffer 全体を drop / 再確保しなくてよい)。
/// - 敗者を clone しなくなる。従来は候補 1 つごとに `value.clone()` してから
///   比較で捨てていたが、clone は winner を [`apply_value`] に渡す 1 回だけになる。
///
/// sibling の [`CascadedDecl`] は tuple alias のままだが、そちらは常に named
/// binding へ destructure され positional access されない。本型は [`beats`] が
/// 3 field を**順序付き比較**するので named field にしてある —
/// [`specificity`](Self::specificity) は `u32` の alias、隣の
/// [`source_order`](Self::source_order) も `u32` であり、tuple の `.1` / `.2`
/// では取り違えても compile が通って cascade の勝敗が静かに壊れる。
#[derive(Clone, Copy)]
struct RankedDecl {
    /// [`cascade_rank`] の origin + `!important` 優先度。
    rank: u8,
    /// selector specificity (inline style は [`INLINE_SPECIFICITY`])。
    specificity: Specificity,
    /// stylesheet 内出現順 (inline style は [`INLINE_SOURCE_ORDER`])。
    source_order: u32,
    /// [`pick_winners`] に渡された `candidates` slice 内の位置。
    idx: usize,
}

/// Cascade origin + `!important` flag に基づく優先度 rank (raikiri-spike-m1.22)。
///
/// 高いほど勝つ。CSS Cascading L4 §6.4.4 の origin 反転扱いを表現:
/// - Normal   : UA < User < Author (Author が最強、UA が最弱)
/// - Important: UA > User > Author (反転、UA が最強)
///
/// M1 では User origin を扱わないので UA + Author の 2 段。
///
/// `@page` cascade (raikiri-spike-m4.1) も同じ origin ordering を共有するため
/// `pub(crate)` で公開し [`crate::page::cascade_page`] から reuse。
pub(crate) fn cascade_rank(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::Author, false) => 1,
        (Origin::Author, true) => 2,
        (Origin::UserAgent, true) => 3,
    }
}

/// `collect_cascaded` は本来 DFS で node を訪れるが、per-node の処理は他の
/// node の状態に依存しないため訪問順は無関係。overflow 回避のため explicit
/// `Vec` stack で iterative に書き換え (roborev job 199)。
fn collect_cascaded<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut HashMap<StyleNodeId, Vec<CascadedDecl>>,
) {
    let mut stack: Vec<StyleNodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            // silent bug fix: 従来 template 内 element にも rule matching が走り
            // Vec<CascadedDecl> が waste で膨らんでいた。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let mut per_node = Vec::new();
                // stylesheet rule matching
                let tag = elem.tag_name();
                for rule in &rule_tree.style_rules {
                    if let Some(spec) = match_by_tag(&rule.selectors, tag) {
                        for decl in &rule.declarations {
                            // shorthand を longhand に展開してから candidate に
                            // 積む (parse 出口の展開だけでは `RuleTree` の
                            // post-parse mutation 経路を守れないため —
                            // bd raikiri-spike-nqkj)。rationale は
                            // `crate::rule::expand_shorthand_into` doc に集約。
                            expand_shorthand_into(decl, |d| {
                                per_node.push((
                                    d.value,
                                    d.important,
                                    rule.origin,
                                    spec,
                                    rule.source_order,
                                ));
                            });
                        }
                    }
                }
                // inline style
                if let Some(source) = elem.inline_style_source() {
                    let mut input = ParserInput::new(source);
                    let mut parser = Parser::new(&mut input);
                    for decl in parse_declaration_block(&mut parser) {
                        per_node.push((
                            decl.value,
                            decl.important,
                            Origin::Author,
                            INLINE_SPECIFICITY,
                            INLINE_SOURCE_ORDER,
                        ));
                    }
                }
                if !per_node.is_empty() {
                    out.insert(id, per_node);
                }
            }
            // stack は LIFO なので document order で push するため reverse。
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
}

/// tag_name 文字列と selector list を突き合わせる簡易 matcher (M1.4 scope)。
///
/// Returns: matching した selector の最大 specificity。1 つも match しなければ None。
/// - `Component::LocalName(name)` — name eq_ignore_ascii_case で判定
/// - `Component::ExplicitUniversalType` — 常に match
fn match_by_tag(list: &SelectorList<RaikiriSelectorImpl>, tag_name: &str) -> Option<Specificity> {
    use selectors::parser::Component;

    let mut best: Option<Specificity> = None;
    for selector in list.slice() {
        let mut matches = true;
        for component in selector.iter_raw_match_order() {
            match component {
                Component::LocalName(local) => {
                    // local.name は Atom (raikiri-style::Atom)、tag_name 文字列と比較
                    if !tag_name.eq_ignore_ascii_case(local.name.0.as_str()) {
                        matches = false;
                        break;
                    }
                }
                Component::ExplicitUniversalType
                | Component::ExplicitAnyNamespace
                | Component::ExplicitNoNamespace
                | Component::DefaultNamespace(_) => {
                    // 常に match / namespace は m1.4 では常に true 扱い
                }
                _ => {
                    // 他 component (class/id/attr/combinator/pseudo) は m1.4 では
                    // ruletree build 段で drop 済のはずだが safety net で match fail
                    matches = false;
                    break;
                }
            }
        }
        if matches {
            let spec = specificity_of(selector);
            best = Some(match best {
                Some(prev) => prev.max(spec),
                None => spec,
            });
        }
    }
    best
}

fn specificity_of(selector: &Selector<RaikiriSelectorImpl>) -> Specificity {
    // selectors crate の Selector::specificity は 32-bit packed integer を返す。
    selector.specificity()
}

/// Top-down inheritance walk。子 node は親の computed value を必要とするため
/// (再帰の call stack で暗黙に運んでいた context)、iterative 化には各 stack
/// entry に `(StyleNodeId, 親の computed value, rem context)` を明示的に持たせる —
/// Approach A (roborev job 199 対応)。clone は各 entry ごとに発生するが m1.4
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
///   computed font-size が `ctx.root_font_size`。
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
fn resolve_inheritance<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    cascaded: &HashMap<StyleNodeId, Vec<CascadedDecl>>,
    out: &mut Vec<ComputedValues>,
) {
    let mut stack: Vec<(StyleNodeId, ComputedValues, Option<ResolveContext>)> =
        vec![(id, parent_computed.clone(), None)];
    // `apply_winners` の scratch buffer。walk loop の**外**で確保して全 node で
    // 使い回す (bd raikiri-spike-8kn8) — per-node の `HashMap` 2 個が
    // n=1000 node で 3,667 allocs / 3.0 MB = cascade 全 heap traffic の 56.7%
    // を占めていた。buffer は最初の数 node で最大 `PropertyKey` index まで
    // 育ち、以降は 0 alloc。fill と drain は `apply_winners` に閉じており、
    // walk loop 側は「使い回す入れ物を貸す」以上の責務を持たない。
    let mut winners: Vec<Option<RankedDecl>> = Vec::new();
    while let Some((id, parent_computed, rem_ctx)) = stack.pop() {
        // raikiri-spike-37c roborev job 294 M2 finding: is_in_document()==false
        // の node は subtree ごと早期 continue する。
        //
        // 以前は resize + write + children push を unconditional に行い computed
        // 長を node_count() に揃えていた (m1.23 contract)。今 `cascade()` が
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

        // phase 1: 親からの inheritance walk 開始値 (inherited のみ親の computed
        // からコピー、非継承は initial) に自 node の cascaded winner を適用する。
        // 適用対象は staging 表現なので winner の適用順に依存しない
        // (spec §M1.4a、raikiri-spike-m1.22 / raikiri-spike-082k)。
        let mut specified = SpecifiedValues::inherit_from(&parent_computed);
        if let Some(candidates) = cascaded.get(&id) {
            apply_winners(candidates, &mut winners, &mut specified);
        }

        // phase 2 + phase 3: 絶対化。root element (element 祖先なし) は `rem` の
        // 基準が phase 2 / phase 3 で異なるため専用 entry point を通す
        // ([`SpecifiedValues::finalize_as_root`] の doc に spec verbatim)。
        let computed = match &rem_ctx {
            Some(ctx) => specified.finalize(parent_computed.font_size, ctx),
            None => {
                // `finalize_as_root` は phase 2 の基準を initial value に固定する
                // (§6.1.1 の "if the element has no parent")。それが正しいのは
                // **`rem_ctx == None` ならこの node に element 親が居ない**からで
                // あり、その caller-side invariant を pin しておく:
                //
                // - `cascade()` は必ず `dom.root_id()` (= Document node) から
                //   walk を開始し、そこに `ComputedValues::initial()` を渡す。
                // - `collect_cascaded` は Element にしか winner を作らないので
                //   Document node の computed は initial のまま。
                // - `rem_ctx` が `None` のままなのは Document 自身とその直接の子
                //   だけ (element を 1 つ通れば `Some` になる)。
                //
                // したがって subtree の途中から `resolve_inheritance` を呼ぶ
                // entry point (incremental restyle 等) を将来足すなら、
                // `rem_ctx` を呼び出し側から供給しなければならない。この assert が
                // その見落としを debug build で捕まえる。
                debug_assert_eq!(
                    parent_computed.font_size,
                    ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
                    "rem_ctx == None は element 親が居ないことを意味するので、\
                     親の computed font-size は initial でなければならない \
                     (subtree の途中から walk を開始していないか?)"
                );
                specified.finalize_as_root()
            }
        };

        // 子へ渡す rem context。root element の phase 2 が終わった時点で
        // `root_font_size` が確定するので、ここで初めて `Some` になる。
        let child_ctx = match rem_ctx {
            Some(ctx) => Some(ctx),
            None if is_element => Some(ResolveContext::new(computed.font_size)),
            None => None,
        };

        // out を id+1 サイズに resize してから index 書き込み。
        // `cascade()` の pre-allocation で通常 out.len() == node_count() のため
        // resize は no-op、defensive safety net として維持 (Dom impl の
        // node_count() 過小報告に対する保険)。
        let idx = id.0 as usize;
        if out.len() <= idx {
            out.resize(idx + 1, ComputedValues::initial());
        }
        out[idx] = computed.clone();

        // 子を stack に push (own computed value を parent_computed として渡す)。
        // stack は LIFO なので document order で push するため reverse。
        let children: Vec<_> = dom.child_ids(id).collect();
        for child_id in children.into_iter().rev() {
            stack.push((child_id, computed.clone(), child_ctx));
        }
    }
}

/// 1 node 分の cascade winner を選び、staging 表現へ適用する (**phase 1**)。
///
/// `winners` は caller が walk loop の外で確保した scratch buffer
/// (bd raikiri-spike-8kn8)。本関数が fill ([`pick_winners`]) と drain を対で
/// 行い、抜けるときは全 slot が `None` に戻っている。
///
/// # 適用順
///
/// slot を index 昇順 = [`PropertyKey`] の**宣言順**に走査する。
/// [`Option::take`] が slot を `None` に戻すので、この走査自体が次 node 用の
/// reset を兼ねる。
///
/// 8kn8 以前は `HashMap` iteration 順 (per-process random seed) だった。既存
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
///   [`crate::rule::expand_shorthand_into`] を通す (bd raikiri-spike-nqkj)。
///   [`RuleTree`] は `pub` field かつ umbrella re-export なので Consumer が
///   parse 後に shorthand を書き戻せる。**parse 出口の展開だけでは守れない**
///   のはこの経路。
///
/// 展開後は同一 key の longhand が複数 candidate になるが、[`beats`] の `>=`
/// が「同 rank/spec/order なら後方勝ち」を与えるので §6.1 の order of appearance
/// がそのまま成立する。
///
/// 残る穴は **compile-time に強制されていない**こと — 展開関数の catch-all arm
/// があるため、新しい shorthand variant の展開 arm を書き忘れても compile error
/// にならず、その shorthand が黙ってここへ流れる。exhaustive 化は
/// **bd raikiri-spike-ez7b** の scope で、展開の match arm は 1 関数に集約されて
/// いるので ez7b が入れば上の 2 経路が同時に保証を得る。それが入るまでの中間
/// guard は `crate::rule` の `declaration_block_never_emits_shorthand_keys`
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
/// [`PropertyKey`]: crate::property::PropertyKey
fn apply_winners(
    candidates: &[CascadedDecl],
    winners: &mut Vec<Option<RankedDecl>>,
    specified: &mut SpecifiedValues,
) {
    pick_winners(candidates, winners);
    for slot in winners.iter_mut() {
        if let Some(winner) = slot.take() {
            apply_value(candidates[winner.idx].0.clone(), specified);
        }
    }
}

/// property key ごとに勝者 declaration を pick (specificity + !important + source order)。
///
/// 結果は返さず `best` に書く。`best` は [`PropertyKey`] の discriminant を
/// そのまま index にした **direct-address table** で、`best[k as usize]` が
/// key `k` の勝者 (= `candidates` 内 index) を持つ。
///
/// # なぜ `HashMap` を返さないのか (bd raikiri-spike-8kn8)
///
/// [`PropertyKey`] は payload を持たない ~40 variant の 1-byte enum、すなわち
/// **既に密な小整数**であり、hash して bucket を引く価値がない。従来実装は
/// per-node に `HashMap` を 2 つ (作業用と戻り値) 建てており、n=1000 node の
/// cascade で 3,667 allocs / 3.0 MB — 全 heap traffic の 56.7% を占めていた。
/// slot 配列にすると allocation は buffer が最大 index まで育つ最初の数 node
/// だけで済み、以降の node は再利用で 0 alloc になる。
///
/// 唯一の caller は [`apply_winners`]。buffer の確保と使い回しは
/// [`resolve_inheritance`] の walk loop が持つ。
///
/// # 呼び出し契約
///
/// - **entry**: `winners` の全 slot が `None` であること (debug_assert で検査)。
/// - **exit**: 出現した key の slot だけが `Some`。
///
/// [`apply_winners`] が fill と drain を対で行うので、通常この契約は自明に
/// 満たされる。debug_assert を残してあるのは、drain loop が unwind
/// (`apply_value` 内 panic) 等で途中終了した場合に slot が生き残る経路が
/// あるため — 次 node がその残骸を拾うと **別 node の declaration を適用**して
/// しまう。安いので保険として置いてある。
///
/// [`PropertyKey`]: crate::property::PropertyKey
fn pick_winners(candidates: &[CascadedDecl], winners: &mut Vec<Option<RankedDecl>>) {
    debug_assert!(
        winners.iter().all(Option::is_none),
        "pick_winners は空の scratch buffer を要求する — \
         前 node の winner slot が生き残っている (drain の unwind 等)"
    );

    for (idx, (value, important, origin, spec, order)) in candidates.iter().enumerate() {
        // fieldless enum の discriminant をそのまま slot index に使う。
        // variant が増えても `resize` が追随するので上限定数は持たない。
        let slot = value.key() as usize;
        if winners.len() <= slot {
            winners.resize(slot + 1, None);
        }
        let candidate = RankedDecl {
            rank: cascade_rank(*origin, *important),
            specificity: *spec,
            source_order: *order,
            idx,
        };
        if winners[slot].is_none_or(|existing| beats(candidate, existing)) {
            winners[slot] = Some(candidate);
        }
    }
}

fn beats(candidate: RankedDecl, existing: RankedDecl) -> bool {
    // Tuple compare: (rank, specificity, source_order)
    // - rank 高い方が勝つ (Important UA > Important Author > Normal Author > Normal UA)
    // - 同 rank なら specificity 高い方が勝つ
    // - 同 rank + spec なら source_order 大 (=後ろ) が勝つ
    // `>=` は同一 rule 内 duplicate property の後方勝ち (CSS Cascading L4 §6.1
    // "Order of Appearance" <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
    // の "the last declaration in document order wins") のため意図的。
    // cross-rule では source_order が異なるので `>=` でも安全。
    //
    // `idx` を比較 key に**入れないこと** — `idx` は candidates 走査順に単調
    // 増加するので第 4 key に足しても結果は変わらないが、tie-break 規則が
    // 「同 rank/spec/order なら後方勝ち」であることが code から読めなくなる。
    (
        candidate.rank,
        candidate.specificity,
        candidate.source_order,
    ) >= (existing.rank, existing.specificity, existing.source_order)
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
/// 可能になった今 (raikiri-spike-5iy)、その 2 行は実際に踏まれる:
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
fn resolve_relative_weight(specified: FontWeightValue, inherited: u16) -> u16 {
    match specified {
        FontWeightValue::Absolute(w) => w,
        FontWeightValue::Bolder => match inherited {
            w if w < 100 => 400,
            w if w < 350 => 400,
            w if w < 550 => 700,
            w if w < 750 => 900,
            w if w < 900 => 900,
            // `900 <= w`: no change (1000 のような 900 超の継承値をそのまま返す)
            w => w,
        },
        FontWeightValue::Lighter => match inherited {
            // `w < 100`: no change (50 のような 100 未満の継承値をそのまま返す)
            w if w < 100 => w,
            w if w < 350 => 100,
            w if w < 550 => 100,
            w if w < 750 => 400,
            w if w < 900 => 700,
            _ => 700,
        },
    }
}

/// specified value を継承元の computed values に対して解決し、**`PropertyValue`
/// 表現のまま** computed-equivalent な値を返す。
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
/// bd raikiri-spike-ygl0 (`@page { font-weight: bolder }` が
/// `FontWeightValue::Bolder` のまま park していた regression) がまさにこの形
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
///    destructure して guard を payload 層に降ろすこと (`FontSize` は
///    bd raikiri-spike-zls8 でそれを済ませた — payload を destructure して
///    `resolve_font_size` に渡している)。
/// 2. **本関数を呼ばない新しい entry point** — ygl0 の regression はこの形
///    だった (`cascade_page` が `apply_value` を通らなかった)。CSS Paged Media 3
///    §6 の margin-box cascade は page context を継承元とする第 3 の経路になる。
///    exhaustive match は「経路の数え上げ」を強制しない。
///
/// # 本関数の pass-through は「解決済」ではない (phase 3 が要る)
///
/// 本関数が担うのは「継承元 computed values **だけ**で解ける」解決に限る =
/// **phase 2**。pass-through arm を通った値のうち、box property
/// ([`padding`](PropertyValue::PaddingTop) /
/// [`margin`](PropertyValue::MarginTop) / [`width`](PropertyValue::Width) /
/// [`height`](PropertyValue::Height) / `border-*-width`) と `line-height` の
/// [`Length`](crate::property::Length) `Em` / `Rem` / `Pt` は**まだ specified
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
/// (`crate::page::absolutize_in_page_context`) を走らせ、そこで page context の
/// font-size を基準に絶対化 + style gating を行う (bd raikiri-spike-sshp)。
/// したがって
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// に届く時点では computed 値になっている — **本関数の戻り値をそのまま public に
/// 出す新しい呼び手を書いてはならない**。
///
/// element 経路の対応物は [`apply_value`] → [`SpecifiedValues::finalize`] で、
/// phase 2 (font-size 確定) → phase 3 (自 font-size 基準で残りを絶対化) が
/// 同じ順に走る。両経路の phase 3 は `crate::resolve` の同じ関数群へ funnel する
/// ので、spec 規則 (`em` / `rem` の基準、percentage の素通し、border style
/// gating) の実装は 1 本ずつしかない。
///
/// # phase 3 でも解けない値 (1 つだけ)
///
/// - [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent)。CSS
///   Text 3 §6.1
///   <https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent> は
///   継承元の computed `text-align` を継承元の `direction` に対して解釈した値を
///   computed value と規定する。**原理的には `inherited` だけで解ける** (=
///   本関数の担当) **が** raikiri は `direction` を computed 層に持たないため
///   未実装 ([`TextAlign`](crate::property::TextAlign) doc の (b) milestone
///   subset carve-out と同じ gap)。phase 3 を足しても解決しない唯一の残り。
///
/// なお `Percent` は「未解決」ではない: box property の computed value は
/// percentage のままである (CSS Values 4 §5.5.1、および §6 の "Percentage values
/// on the margin and padding properties are relative to the dimensions of the
/// containing block" = used 層の入力)。element 経路の
/// [`crate::resolve::resolve_length_percentage`] と同じ扱い。
pub(crate) fn resolve_against_inherited(
    value: PropertyValue,
    inherited: &ComputedValues,
) -> PropertyValue {
    match value {
        // CSS Fonts 4 §2.2.1 "Relative Weights"
        // <https://www.w3.org/TR/css-fonts-4/#relative-weights>: `bolder` /
        // `lighter` は継承元の computed weight に対して解決される。ここで
        // `Absolute` に落とすので戻り値に relative keyword は残らない
        // (`Absolute(u16)` → `u16` → `Absolute(u16)` の round-trip は無損失)。
        PropertyValue::FontWeight(fw) => PropertyValue::FontWeight(FontWeightValue::Absolute(
            resolve_relative_weight(fw, inherited.font_weight),
        )),
        // `font-size` は継承元の computed font-size だけで解ける (bd
        // raikiri-spike-zls8)。`crate::resolve::resolve_font_size` に funnel し、
        // 結果を `Length::Px` で包み直して computed-equivalent にする
        // (`FontWeight` arm が `Absolute(u16)` を返すのと同じ形)。
        //
        // 基準が `inherited.font_size` でよい根拠:
        //
        // - `em`: CSS Paged Media 3 §6 "Page Properties"
        //   <https://www.w3.org/TR/css-page-3/#page-properties> verbatim —
        //   "When used on the font-size property in the page context, they are
        //   relative to the font-size of the root element."
        //   `cascade_page` の `inherited` は root element の `ComputedValues`
        //   そのもの (無い場合は initial) なので、これが §6 の言う基準である。
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
        //
        // 本 arm が `font-size` に限る理由: box property
        // (`padding` / `margin` / `width` / `height` / `border-*-width`) の
        // `em` は page context 自身の font-size を要し、それは同 cascade の兄弟
        // declaration から来得るので `inherited` だけでは決まらない。それらは
        // 呼び手 (`cascade_page`) が本関数の後に走らせる phase 3 の担当である
        // (上の「本関数の pass-through は『解決済』ではない」節を参照)。
        // **本 arm の戻り値が常に `Length::Px` であることは load-bearing** —
        // phase 3 はその値を page context の font-size (= `em` の基準) として
        // 読み戻す (`crate::page::page_context_font_size`)。
        PropertyValue::FontSize(len) => PropertyValue::FontSize(Length::Px(
            crate::resolve::resolve_font_size(
                len,
                inherited.font_size,
                &ResolveContext::new(inherited.font_size),
            )
            .px(),
        )),
        // 本関数では解決しない property — pass-through。上記 doc の「本関数の
        // pass-through は『解決済』ではない (phase 3 が要る)」節が、これらを
        // 呼び手の phase 3 が絶対化することを説明している。`_` に潰さないこと。
        v @ (PropertyValue::Color(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::LineHeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::TextAlign(_)
        | PropertyValue::PaddingTop(_)
        | PropertyValue::PaddingRight(_)
        | PropertyValue::PaddingBottom(_)
        | PropertyValue::PaddingLeft(_)
        | PropertyValue::Padding(_)
        | PropertyValue::MarginTop(_)
        | PropertyValue::MarginRight(_)
        | PropertyValue::MarginBottom(_)
        | PropertyValue::MarginLeft(_)
        | PropertyValue::Margin(_)
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
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::BoxSizing(_)) => v,
    }
}

/// Cascade winner 1 つを staging 表現 ([`SpecifiedValues`]) に書き込む
/// (**phase 1**)。
///
/// length を運ぶ property は **specified 表現のまま**格納する — 絶対化は
/// [`SpecifiedValues::finalize`] (phase 2 + phase 3) の責務であり、本関数の中で
/// 行うことは decision raikiri-spike-082k により禁じられている (`padding: 2em` の
/// 基準となる font-size は**その node の全 winner を適用し終える**まで確定せず、
/// 本関数は winner 1 つ分しか見ていないため)。
///
/// 例外は `font-weight` — `bolder` / `lighter` は**継承元**の computed weight
/// だけで解ける (自 node の他 winner に依存しない) ため、ここで絶対値に落とす。
/// 詳細は該当 arm の comment を参照。
fn apply_value(value: PropertyValue, target: &mut SpecifiedValues) {
    match value {
        PropertyValue::Color(c) => target.color = c,
        // CSS Backgrounds 3 §2.2 (raikiri-spike-0vv.7)。sibling `Color` と対称的な
        // 単純代入 (non-inherited、per-node で cascade winner を直接反映)。
        PropertyValue::BackgroundColor(c) => target.background_color = c,
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        // CSS Fonts 4 §2.2 (raikiri-spike-5iy + raikiri-spike-17s8)。specified
        // value は `FontWeightValue` (relative keyword を保持)、computed value
        // は resolve 済み `u16` — `bolder` / `lighter` はここで絶対値に落とす。
        //
        // 継承値の出所: `target` は直前に `SpecifiedValues::inherit_from(parent)`
        // で seed されており (`resolve_inheritance` 参照)、`font_weight` は
        // inherited property なので **この時点の `target.font_weight` は親の
        // computed font-weight そのもの**。`SpecifiedValues` が
        // `font_weight: u16` を「既に computed-equivalent」として持つのはこの
        // invariant のため — `SpecifiedValues::initial()` から seed する実装に
        // 変えると `bolder` が常に 400 起点になり、compile error にも既存 test の
        // 失敗にもならずに壊れる (bd raikiri-spike-i5bs §8.2 debt lens D5)。
        // さらに `pick_winners` は
        // `PropertyKey` ごとに slot を 1 つだけ埋めるため `FontWeight` arm が同一
        // node で 2 回走ることはなく、winner の適用順にも依存しない
        // (`winner_does_not_leak_into_next_sibling` test がこの "1 回だけ" を pin
        // する — 二重適用は 400 → 700 → 900 と複合するので観測可能)。
        // この 2 つが relative-weight resolution の正しさを支える invariant。
        //
        // なお本 arm は `apply_value` 中で **唯一の read-modify-write** (他は
        // すべて冪等な単純代入)。`Padding` / `Margin` / `Border` arm のような
        // "safety net" 二重適用経路を font-weight に足すと `bolder` が
        // 400 → 700 → 900 と複合するため、上記 2 invariant を崩す変更は不可。
        //
        // **契約 (raikiri-spike-ygl0)**: 継承元依存の解決を持つ property を新しく
        // 追加するときは、本 arm だけでなく sibling の
        // [`resolve_against_inherited`] にも arm を足すこと — そちらは
        // `PropertyValue` を返す形で同じ解決を提供し、
        // [`crate::page::cascade_page`] (`apply_value` を通らない第 2 の public
        // entry point) が使う。両者とも wildcard 無しの exhaustive match なので
        // variant 追加時は compiler が 2 経路を数え上げさせる。
        PropertyValue::FontWeight(fw) => {
            let inherited = target.font_weight;
            target.font_weight = resolve_relative_weight(fw, inherited);
        }
        // CSS Inline 3 §5.1: line-height は inherited、cascade winner が
        // raw value (Normal / Number / Length) を保持。number-vs-length
        // distinction は下流 (paint) の resolve context で意味を持つ
        // (raikiri-spike-0vv.9)。
        PropertyValue::LineHeight(lh) => target.line_height = lh,
        PropertyValue::Display(d) => target.display = d,
        // counter-* は M5 pre-work (raikiri-spike-s85) — parse 結果をそのまま
        // computed value に格納。counter tree resolve は M5 本編。
        PropertyValue::CounterReset(v) => target.counter_reset = v,
        PropertyValue::CounterIncrement(v) => target.counter_increment = v,
        PropertyValue::CounterSet(v) => target.counter_set = v,
        // content は M5 gcpm-directive-emit static-side (raikiri-spike-m5.1)。
        // 下流 (raikiri-dom) runtime resolve が counter()/string()/target-*() の
        // 実値を組み立てる際に本 field を参照。
        PropertyValue::Content(v) => target.content = v,
        // string-set は M5 static-side β (raikiri-spike-m5.3、CSS GCPM 3 §3.1)。
        // Named-string runtime resolve は下流 (raikiri-dom) 責務。
        PropertyValue::StringSet(v) => target.string_set = v,
        // position は M5 static-side ε (raikiri-spike-m5.4、CSS GCPM 3 §1.2.1)。
        // - `Static` は no-op: `inherit_from` が running_templates を空で初期化
        //   するため、`position: static` が cascade winner のとき running_templates
        //   は空のままで正しい (advisor calibration: 先行 running(hdr) を上書きして
        //   template 登録を suppress する用途)。
        // - `Running(name)` は 1-item seed を push。per-node で常に 0/1 要素
        //   (position は spec 上 単一値)、per-document 集約は下流 (raikiri-dom)
        //   の 2-tier キャッシュ static side 責務 (design doc §7.3)。
        PropertyValue::Position(pv) => match pv {
            PositionValue::Static => {}
            PositionValue::Running(name) => {
                target.running_templates.push(RunningTemplate { name });
            }
        },
        // text-align は Sprint 12 seed (raikiri-spike-0vv.8、CSS Text 3 §6.1)。
        // inherited property のため cascade winner が無い child は inherit_from で
        // 親値を引き継ぐ (color / font_family / font_size / font_weight と同じ
        // handling)。TextAlign は Copy、by-value 代入で十分。
        PropertyValue::TextAlign(t) => target.text_align = t,
        // CSS Box 3 §6.1 padding physical longhand (raikiri-spike-0vv.6)。
        // 4 side を独立に上書き。shorthand `PropertyValue::Padding` は
        // `crate::rule::expand_shorthand_into` により parse 出口と cascade 入口
        // (`collect_cascaded`) の両方で 4 longhand に展開されるため、cascade 段に
        // 届く declaration は per-side longhand
        // のみ = shorthand/longhand の cross-key dependency が消え、winner の
        // 適用順に依存しない per-key determinism が成立する
        // (raikiri-spike-5nc、margin 0vv.5 の parse-time expansion model に migrate)。
        PropertyValue::PaddingTop(v) => target.padding.top = v,
        PropertyValue::PaddingRight(v) => target.padding.right = v,
        PropertyValue::PaddingBottom(v) => target.padding.bottom = v,
        PropertyValue::PaddingLeft(v) => target.padding.left = v,
        // CSS Box 3 §6.2 padding shorthand fall-through (normal flow では展開済み)。
        // sibling `PropertyValue::Margin` arm と同じく **safety net ではない** —
        // 到達すれば 4 longhand winner を破壊し spec と食い違う。
        // Sides<Length>: Copy のため move で `target.padding` に代入。
        PropertyValue::Padding(sides) => target.padding = sides,
        // 4 longhand margin sides (raikiri-spike-0vv.5、CSS Box 3 §3.1)。
        // shorthand `PropertyValue::Margin` は `crate::rule::expand_shorthand_into`
        // により parse 出口と cascade 入口の両方で 4 longhand に展開されるため、
        // cascade 段に届く declaration
        // は per-side longhand のみ = winner の適用順に依存しない per-key
        // determinism が成立する (詳細は `crate::rule::expand_shorthand_into` doc)。
        PropertyValue::MarginTop(v) => target.margin.top = v,
        PropertyValue::MarginRight(v) => target.margin.right = v,
        PropertyValue::MarginBottom(v) => target.margin.bottom = v,
        PropertyValue::MarginLeft(v) => target.margin.left = v,
        // ⚠️ **これは "safety" net ではない** (bd raikiri-spike-8kn8 で framing 訂正)。
        // element cascade 経由では到達不能 — `collect_cascaded` が candidate を
        // 積む前に、stylesheet rule は `expand_shorthand_into` で、inline style は
        // `parse_declaration_block` (内部で同関数を呼ぶ) で展開されるため (nqkj)。
        // 残る到達手段は crate 内から `apply_value` を直接呼ぶことだけである。
        // 仮に到達すると、`PropertyKey` 宣言順では `Margin` が `MarginTop` 等より
        // **後**に適用される。したがってこの atomic 上書きは **longhand winner を
        // 必ず破壊する** — `margin: 0; margin-top: 10px` が top=0 になり
        // CSS Cascading L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand> と
        // 食い違う。net は degraded ではなく **deterministic に spec 違反**。
        // 到達した時点で既に bug であり、本 arm はそれを穏当に見せない。
        //
        // それでも `unreachable!` を採らないのは reviewer-security の panic
        // surface 排除方針による。cascade 経路で unreachable なのは
        // `crate::rule::expand_shorthand_into` の 2 call site が担保しており、その
        // 担保が compile-time に強制されていない件と恒久 fix は同関数 doc を参照
        // (bd raikiri-spike-ez7b)。振る舞い自体は
        // `apply_value_direct_margin_shorthand_safety_net` test が直接叩いて pin。
        PropertyValue::Margin(sides) => target.margin = sides,
        // CSS Backgrounds 3 §5.1/§5.2/§5.3 border physical longhand
        // (raikiri-spike-0vv.12)。4 side × 3 sub-property の 12 arm。shorthand
        // `PropertyValue::Border` は `crate::rule::expand_shorthand_into` により
        // parse 出口と cascade 入口の両方で 12 longhand に展開されるため、
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
        // 恒久 fix (bd raikiri-spike-ez7b) は `Margin` arm の comment 参照。
        PropertyValue::Border(sides) => target.border = sides,
        // CSS Sizing 3 §3.1.1 width (raikiri-spike-0vv.10)。single-value property、
        // `LengthOrAuto` は Copy shape (Length variant は Copy)。sibling
        // `PropertyValue::TextAlign` と対称的な単純代入 (non-inherited、cascade
        // winner を直接反映)。`auto` は下流 layout の automatic size calculation
        // (CSS Sizing 3 §5) で解決される — margin `auto` の余白分配とは別意味。
        PropertyValue::Width(v) => target.width = v,
        // CSS Sizing 3 §3.1.1 height (raikiri-spike-0vv.11)。sibling `Width` /
        // `Padding*` / `Margin*` と同じ per-node winner 直接代入 (non-inherited、
        // `LengthOrAuto` は Copy)。resolve (`Percent` / `Auto` の実 layout 高さ
        // 計算) は下流責務。
        PropertyValue::Height(v) => target.height = v,
        // CSS Sizing 3 §3.3 box-sizing (raikiri-spike-0vv.13)。non-inherited、
        // cascade winner が specified keyword をそのまま computed value に反映。
        // BoxSizing は Copy、by-value 代入で十分。
        PropertyValue::BoxSizing(bs) => target.box_sizing = bs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::CssColor;
    use crate::property::DisplayValue;
    use crate::property::{Border, BorderColor, BorderStyle, Length, LengthOrAuto, Sides};
    use crate::resolve::{
        ComputedBorder, ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
        ComputedLineHeight,
    };
    use crate::ruletree::build_rule_tree;
    use crate::test_dom::TestDoc;
    use smol_str::SmolStr;

    fn cascade_doc(css: &str, tag: &str, inline: Option<&str>) -> ComputedValues {
        let mut doc = TestDoc::new();
        if !css.is_empty() {
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, css);
        }
        let e = doc.push_element(0, tag, inline);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        result.computed[e].clone()
    }

    const RED: CssColor = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    const BLUE: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };

    #[test]
    fn empty_dom_root_has_initial() {
        let doc = TestDoc::new();
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        assert_eq!(r.computed[0], ComputedValues::initial());
    }

    #[test]
    fn type_selector_applies_color() {
        let cv = cascade_doc("p { color: red }", "p", None);
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn inline_style_beats_type_selector() {
        let cv = cascade_doc("p { color: red }", "p", Some("color: blue"));
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn type_selector_beats_universal() {
        let cv = cascade_doc("* { color: red } p { color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn important_beats_normal_across_specificity() {
        // universal !important が type normal に勝つ
        let cv = cascade_doc("p { color: red } * { color: blue !important }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn source_order_tiebreak_later_wins() {
        let cv = cascade_doc("p { color: red } p { color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn later_duplicate_in_same_rule_wins() {
        // 同一 rule 内で同じ property が 2 回 — CSS §6.4.4: 後方の declaration が勝つ。
        let cv = cascade_doc("p { color: red; color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn later_duplicate_in_inline_wins() {
        // inline style 内で同じ property が 2 回 — 同様に後方が勝つ。
        let cv = cascade_doc("", "p", Some("color: red; color: blue"));
        assert_eq!(cv.color, BLUE);
    }

    /// `pick_winners` の scratch buffer は walk loop の外で確保され全 node で
    /// 共有される (bd raikiri-spike-8kn8)。**この共有が持ち込む唯一の新しい
    /// 失敗様式が「前 node の winner slot が drain されずに残り、次 node へ
    /// 漏れる」**であり、本 test がそれを pin する。
    ///
    /// 兄弟 2 つに **互いに素な property** を当てるのが要点:
    /// `<p>` は `background-color` slot (discriminant 1) だけを、`<span>` は
    /// `font-weight` slot (discriminant 4) だけを埋める。
    ///
    /// # なぜ `font-weight: bolder` なのか
    ///
    /// slot が持つのは値そのものではなく **`candidates` 内 index** なので、
    /// 漏れた slot は「次 node の candidate list を誤った index で読む」形で
    /// 顕在化する。ここでは `<span>` の `candidates[0]` が `font-weight:
    /// bolder` なので、`<p>` の残した slot 1 と自分の slot 4 が**同じ
    /// declaration を 2 回**適用する。`bolder` は [`apply_value`] 中で唯一の
    /// read-modify-write arm であるため 400 → 700 → **900** と複合し、
    /// 期待値 700 とずれる。単純代入 property を選ぶと二重適用が冪等になって
    /// leak を素通ししてしまう (実際 `padding` で書いた初版は、drain の
    /// `take()` を `*slot` に落とす mutant を release build で検出できなかった)。
    ///
    /// `background_color` 側の assertion は構造的な control で、こちらは
    /// **non-inherited** であることが効いている — `color` のような inherited
    /// property では「親から継承した値」と「兄弟から漏れた値」が区別できない。
    ///
    /// # 本 test の射程
    ///
    /// - leak の**向き**は traversal 順に依存するので、検出できるのは
    ///   document order で先行する `<p>` → 後続 `<span>` の向きだけ。
    /// - `cargo test` は debug build なので、実際に leak すると
    ///   [`pick_winners`] 冒頭の debug_assert が先に落ちる。`bolder` の二重適用
    ///   機構が単独で load-bearing になるのは **release build** (debug_assert が
    ///   消える) のみ。逆に言えば本 test の価値は release build での検出可能性と、
    ///   失敗時の診断 message の明示性にある。
    #[test]
    fn winner_does_not_leak_into_next_sibling() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "p { background-color: red } span { font-weight: bolder }",
        );
        let wrapper = doc.push_element(0, "div", None);
        let p = doc.push_element(wrapper, "p", None);
        let span = doc.push_element(wrapper, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();

        let initial = ComputedValues::initial();
        assert_eq!(r.computed[p].background_color, RED);
        assert_eq!(
            r.computed[span].background_color, initial.background_color,
            "span に p の background-color winner が漏れた"
        );
        // 親 <div> は initial の 400。CSS Fonts 4 §2.2.1 の表で
        // 350 <= 400 < 550 → bolder = 700。二重適用なら 900 になる。
        assert_eq!(
            r.computed[span].font_weight, 700,
            "font-weight: bolder が 2 回適用された (slot leak による二重 drain)"
        );
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

    #[test]
    fn cascade_result_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<CascadeResult>();
    }

    // ── 絶対化 (082k Phase 2 / bd raikiri-spike-zls8) ───────────────────
    //
    // ここから下の test 群は「cascade を抜けた時点で length が px に解決されて
    // いる」ことを pin する。従来 (Sprint 18 まで) は specified value が
    // `ComputedValues` に素通りし、`em` / `rem` は下流 (raikiri-dom layout.rs)
    // で黙って 0px に潰れていた。

    /// 2 段の element を作り、両者の [`ComputedValues`] を返す。
    ///
    /// `cascade_doc` は `<style>` と対象 element を **兄弟**として Document 直下に
    /// 置くため、親子関係を要する test (inheritance / `rem` の root element 判定)
    /// には使えない。
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

    /// decision raikiri-spike-082k Rationale 1 — **本 task の存在理由**。
    ///
    /// 従来はどちらの `<span>` も `font_size == Length::Em(1.5)` になり、
    /// 「宣言由来の `em`」と「inherit 由来の値」が区別できなかった (下流から
    /// 修復不能な live bug)。CSS Cascade 5 §7.2
    /// (<https://www.w3.org/TR/css-cascade-5/#inheriting>) が「inheritance が
    /// 運ぶのは computed value である」と規定するため、両者は別の値でなければ
    /// ならない。
    ///
    /// - (i) `<div style="font-size:1.5em"><span style="font-size:1.5em">` →
    ///   span は **36px** (自 declaration が親 24px に対して解決)
    /// - (ii) `<div style="font-size:1.5em"><span>` → span は **24px**
    ///   (親の computed value を継承)
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

    /// `em` の compounding が cascade 経由で成立する (16px → 1.5em → 1.5em)。
    /// CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#em>。
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

    /// `font-size` 以外の length は **自 node の** computed font-size 基準
    /// (CSS Values 4 §6.1.1 `em`)。従来は下流で 0px に潰れていた経路。
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

    /// **root element 自身の `Nrem`** は initial value (16px) 基準。
    ///
    /// CSS Values 4 §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>): "…or
    /// against the computed metrics corresponding to the initial values of the
    /// font and line-height properties, **if the element has no parent**."
    ///
    /// この case は `resolve_inheritance` が root element を過小に special-case
    /// した場合 (tree 全体に単一の `ResolveContext::new(root_font_size)` を配る
    /// 誤実装) に落ちる — 自己参照になり `2rem` が発散/固定点に落ちる。
    #[test]
    fn rem_on_root_element_resolves_against_initial_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: 2rem"));
        assert_eq!(cv.font_size, ComputedLength(32.0));
    }

    /// **root element の子の `Nrem`** は root element の computed font-size 基準。
    ///
    /// CSS Values 4 §6.1.1 (<https://www.w3.org/TR/css-values-4/#rem>): `rem` —
    /// "Equal to the computed value of the em unit on the root element."
    ///
    /// この case は root element を **過剰に** special-case した場合 (子まで
    /// `ResolveContext::initial()` を配る誤実装) に落ちる — 40px ではなく 32px に
    /// なる。上の test と対で初めて閉じる。
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

    /// root element 上の **box property** の `rem` は自 font-size 基準
    /// (font-\* property ではないので CSS Values 4 §6.1.1 の parent-metrics
    /// 条項は発火せず、`rem` は素の定義「root element の computed font-size」に
    /// なる)。
    ///
    /// 上 2 test と合わせて root element の `rem` を 2 方向から挟む — phase 2 は
    /// initial 16px 基準 (32px)、phase 3 は自 20px 基準 (40px)。
    /// **root 全体に `ResolveContext::initial()` を配る実装ではここが 32px に
    /// なって落ちる。**
    #[test]
    fn rem_on_root_element_box_property_uses_own_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: 20px; padding: 2rem"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
    }

    /// Document 直下の **非 element** node は rem context を確定させない
    /// (`resolve_inheritance` の `child_ctx` の `None => None` arm)。
    ///
    /// [`StyleDom::root_id`] は Document node であって root element ではないので、
    /// element を 1 つも通っていない経路では `rem` の参照値が未確定のままで
    /// なければならない。Text / Comment node が誤って「root element」扱いされると
    /// 兄弟 element より先に walk された場合に rem 基準が汚染される。
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

    /// `rem` は **root element** 基準であって直近の親基準ではない。
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

    /// **D5 invariant** (bd raikiri-spike-i5bs §8.2 debt lens D5)。
    ///
    /// `bolder` は `SpecifiedValues` の staging 上で解決されるが、その基準は
    /// **親の computed font-weight** でなければならない (CSS Fonts 4 §2.2.1
    /// <https://www.w3.org/TR/css-fonts-4/#relative-weights>)。
    /// `SpecifiedValues::inherit_from` が `font_weight` を親からではなく
    /// `initial()` (400) から seed すると 400 → 700 になり、この test だけが
    /// 落ちる (compile error にはならない)。
    #[test]
    fn bolder_resolves_against_parent_computed_weight_through_staging() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-weight: 700"),
            "span",
            Some("font-weight: bolder"),
        );
        assert_eq!(parent.font_weight, 700);
        assert_eq!(
            child.font_weight, 900,
            "bolder は親の computed 700 に対して解決される (initial 400 起点なら 700 になる)"
        );

        // lighter 側も同じ経路を通る (700 → 400)。
        let (_, lighter) = cascade_parent_child(
            "div",
            Some("font-weight: 700"),
            "span",
            Some("font-weight: lighter"),
        );
        assert_eq!(lighter.font_weight, 400);
    }

    /// 3 段の `bolder` chain — staging を経ても compounding が spec table どおり
    /// 進む (400 → 700 → 900 → 900)。
    #[test]
    fn bolder_chain_compounds_through_staging() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-weight: bolder"));
        let b = doc.push_element(a, "div", Some("font-weight: bolder"));
        let c = doc.push_element(b, "div", Some("font-weight: bolder"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].font_weight, 700);
        assert_eq!(r.computed[b].font_weight, 900);
        assert_eq!(r.computed[c].font_weight, 900);
    }

    /// `pt` は cascade 段で px に絶対化される (CSS Values 4 §6.2
    /// <https://www.w3.org/TR/css-values-4/#absolute-lengths>、`1pt = 4/3px`)。
    ///
    /// **font-size の期待値は initial (16px) と一致させてはならない** — 一致させると
    /// `parse_font_size` が `pt` を drop しても (= 本 commit が受理可能にした経路が
    /// 壊れても) initial が残って pass してしまう。`15pt = 20px` を使う。
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

    /// `line-height: <percentage>` は **宣言要素**で絶対化され、子は length を
    /// そのまま継承する (CSS Inline 3 §5.1
    /// <https://www.w3.org/TR/css-inline-3/#propdef-line-height>
    /// "Percentages: computed relative to 1em" + "Computed value: … a computed
    /// `<length>` value")。子の font-size で再解決してはならない。
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

    /// `line-height: <number>` は computed 層でも number のまま継承され、
    /// **子自身の** font-size に掛かる余地を残す (§5.1 の special behavior)。
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

    /// cascade winner の適用順が絶対化の基準に影響しない
    /// — decision 082k の拘束事項 (絶対化を winner 適用と別 phase にした理由)。
    /// declaration の並び順を入れ替えても `padding: 2em` は同じ 40px になる。
    #[test]
    fn absolutization_is_independent_of_declaration_order() {
        let a = cascade_doc("", "div", Some("font-size: 20px; padding: 2em"));
        let b = cascade_doc("", "div", Some("padding: 2em; font-size: 20px"));
        assert_eq!(a.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
        assert_eq!(a.padding, b.padding);
        assert_eq!(a.font_size, b.font_size);
    }

    #[test]
    fn cascade_deterministic_across_10_runs() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red } * { color: blue !important }");
        let p = doc.push_element(0, "p", Some("font-size: 20px"));
        doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);

        let baseline = cascade(&doc, &tree).unwrap();
        for _ in 0..9 {
            let run = cascade(&doc, &tree).unwrap();
            assert_eq!(run.computed.len(), baseline.computed.len());
            for i in 0..run.computed.len() {
                assert_eq!(run.computed[i], baseline.computed[i], "differ at node {i}");
            }
        }
    }

    fn deep_chain_doc(depth: usize) -> (TestDoc, usize) {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
        }
        let style = doc.push_element(parent, "style", None);
        doc.push_text(style, "div { color: red }");
        (doc, parent)
    }

    /// roborev job 199 (medium): `collect_cascaded` and `resolve_inheritance`
    /// were recursive DFS — a deeply nested DOM could stack-overflow the
    /// process. 5000-level linear chain must cascade without overflow and
    /// produce a correct (non-initial) computed value at the deepest node.
    #[test]
    fn deep_nesting_5000_cascade_no_overflow() {
        let (doc, deepest) = deep_chain_doc(5000);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed.len(), doc.nodes.len());
        assert_eq!(result.computed[deepest].color, RED);
    }

    /// Same regression, but forces the overflow deterministically: run on a
    /// thread with a small, fixed stack size so recursion depth needed to
    /// blow the stack is low and hardware/platform-independent. Before the
    /// iterative-DFS fix this thread aborts with a stack overflow; after the
    /// fix it completes cleanly and returns the correct computed color.
    #[test]
    fn deep_nesting_small_stack_no_overflow() {
        let handle = std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let (doc, deepest) = deep_chain_doc(500);
                let tree = build_rule_tree(&doc);
                let result = cascade(&doc, &tree).expect("cascade Ok");
                result.computed[deepest].color
            })
            .expect("spawn thread");
        let color = handle
            .join()
            .expect("thread must not stack-overflow on deep DOM");
        assert_eq!(color, RED);
    }

    // ── UA origin + display cascade (M1.4a、raikiri-spike-m1.22) ──

    fn cascade_with_ua(
        ua_css: &str,
        author_css: &str,
        target_tag: &str,
        inline: Option<&str>,
    ) -> ComputedValues {
        // UA rule + Author rule + inline を一気に組み立てて cascade 実行
        let mut doc = TestDoc::new();
        // Author の <style> は DOM 側から build_rule_tree に読ませる
        if !author_css.is_empty() {
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, author_css);
        }
        let e = doc.push_element(0, target_tag, inline);

        // build_rule_tree (Author 集約) + UA add_stylesheet (m1.22 経路)
        let mut tree = build_rule_tree(&doc);
        // UA CSS を先頭に inject するのではなく、既存の Author rule の後ろに
        // add してから rank 化で origin 順序を担保する (source_order より rank
        // が優位)
        // ただし M1.4a では add_stylesheet の呼び出し順で source_order が振られ
        // Author が先 (source_order 小)、UA が後 (source_order 大) となる。
        // rank 化により Origin::UserAgent の Normal は rank=0 (最弱)、
        // Origin::Author の Normal は rank=1 なので UA rule が Author を上書き
        // することはない (source_order に関わらず rank が優先)。
        if !ua_css.is_empty() {
            tree.add_stylesheet(ua_css, Origin::UserAgent);
        }
        let result = cascade(&doc, &tree).expect("cascade Ok");
        result.computed[e].clone()
    }

    #[test]
    fn ua_display_block_applied_when_no_author_rule() {
        // UA CSS のみで <p> の display が Block になる
        let cv = cascade_with_ua("p { display: block }", "", "p", None);
        assert_eq!(cv.display, DisplayValue::Block);
    }

    #[test]
    fn author_display_inline_overrides_ua_block() {
        // Normal Author > Normal UA (rank 1 > rank 0)
        let cv = cascade_with_ua("p { display: block }", "p { display: inline }", "p", None);
        assert_eq!(cv.display, DisplayValue::Inline);
    }

    #[test]
    fn important_ua_beats_important_author_display() {
        // Important UA > Important Author (rank 3 > rank 2、!important 反転)
        let cv = cascade_with_ua(
            "p { display: block !important }",
            "p { display: inline !important }",
            "p",
            None,
        );
        assert_eq!(cv.display, DisplayValue::Block);
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

    // ── background-color wire-through (CSS Backgrounds 3 §2.2、raikiri-spike-0vv.7) ──

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
        // 37n sibling pattern (display / counter-* / content / string-set /
        // position の non-inheritance test 群を踏襲、raikiri-spike-0vv.7)。
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
        // per-node に到達することを pin (parse_color の transparent Ident branch
        // と CssColor::TRANSPARENT const の regression canary、raikiri-spike-0vv.7)。
        let cv = cascade_doc("", "div", Some("background-color: transparent"));
        assert_eq!(cv.background_color, CssColor::TRANSPARENT);
        assert_eq!(cv.background_color.a, 0);
    }

    // ── line-height wire-through (CSS Inline 3 §5.1、raikiri-spike-0vv.9) ──

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

    // ── counter-* wire-through (CSS Lists 3 §3、raikiri-spike-s85 M5 pre-work) ──

    #[test]
    fn counter_reset_wired_through_cascade_from_inline_style() {
        // <div style="counter-reset: chapter"> → ComputedValues.counter_reset
        // に [("chapter", 0)] が届く。parser → PropertyValue → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。
        // d9y.2: counter_reset は Arc<Vec<..>>、`*cv.counter_reset` で deref-compare。
        let cv = cascade_doc("", "div", Some("counter-reset: chapter"));
        assert_eq!(*cv.counter_reset, vec![(SmolStr::new("chapter"), 0)]);
        // 他 counter property は non-inherited の initial (empty) のまま
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
    }

    // ── content wire-through (CSS Content 3 §2、raikiri-spike-m5.1) ──

    #[test]
    fn content_wired_through_cascade_from_inline_style() {
        // <p style='content: "hello"'> → ComputedValues.content に
        // [Literal("hello")] が届く。parser → PropertyValue::Content →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // s85 counter-* wire-through pattern を踏襲。
        // d9y.1: content は Arc<Vec<..>>、`*cv.content` で deref-compare。
        // Literal は SmolStr payload (owned String → SmolStr conversion)。
        use crate::property::ContentComponent;
        use smol_str::SmolStr;
        let cv = cascade_doc("", "p", Some(r#"content: "hello""#));
        assert_eq!(
            *cv.content,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    // ── string-set wire-through (CSS GCPM 3 §3.1、raikiri-spike-m5.3) ──

    #[test]
    fn string_set_wired_through_cascade_from_inline_style() {
        // <p style='string-set: chapter_title "hello"'> → ComputedValues.string_set
        // に [(chapter_title, [Literal("hello")])] が届く。
        // parser → PropertyValue::StringSet → apply_value → ComputedValues の
        // end-to-end 疎通 smoke。s85 / m5.1 wire-through pattern を踏襲。
        // d9y.1: string_set は Arc<Vec<..>>、Literal は SmolStr。indexing +
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
        // spec §3.1: string-set は non-inherited。<p style='string-set: a "x"'>
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

    // ── position: running() wire-through (CSS GCPM 3 §1.2.1、raikiri-spike-m5.4) ──

    #[test]
    fn running_template_wired_through_cascade_from_inline_style() {
        // <div style="position: running(header)"> → ComputedValues.running_templates
        // に [RunningTemplate{name:"header"}] が届く。parser → PropertyValue::Position
        // → apply_value → ComputedValues の end-to-end 疎通 smoke。
        // s85 / m5.1 / m5.3 wire-through pattern を踏襲。
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
        // CSS GCPM 3 §1.2.1 (+ CSS Positioned Layout §9.1.1): position は
        // non-inherited。<div style="position: running(hdr)"> の子 <span> は
        // 自身の rule がなく running_templates は initial (empty)。
        // 37n sibling: string_set / content non-inherited と同じ shape。
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
        // advisor calibration: `Static` variant の load-bearing 検証。
        // 同一 declaration block 内で `position: running(hdr); position: static`
        // → CSS §6.4.4 で後方 declaration が同 rank/spec/order で勝つ (source_order
        // が同じでも `beats` の `>=` で最後の候補が上書きする)。winner は
        // Position(Static)、apply_value は no-op → running_templates 空。
        let cv = cascade_doc("", "div", Some("position: running(hdr); position: static"));
        assert!(
            cv.running_templates.is_empty(),
            "later `position: static` must suppress earlier `running(hdr)` — \
             running_templates should stay empty when Static wins the cascade"
        );
    }

    // ── text-align wire-through + inheritance (CSS Text 3 §6.1、raikiri-spike-0vv.8) ──

    #[test]
    fn text_align_wired_through_cascade_from_inline_style() {
        // <p style="text-align: center"> → ComputedValues.text_align に
        // TextAlign::Center が届く。parser → PropertyValue::TextAlign →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // s85 counter-* / m5.1 content / m5.3 string-set / m5.4 position wire-through
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
        // p に UA-like rule として display: block を Author 側で置く (M1.4a scope
        // では UA rule も同 rank に居るので、child が inherit しない性質だけを見る)
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
            "display must NOT inherit (CSS §9.2.4 non-inherited) — initial Inline"
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

    // ── box-sizing wire-through (CSS Sizing 3 §3.3、raikiri-spike-0vv.13) ──

    #[test]
    fn box_sizing_wired_through_cascade_from_inline_style() {
        // <p style="box-sizing: border-box"> → ComputedValues.box_sizing に
        // BoxSizing::BorderBox が届く。parser → PropertyValue::BoxSizing →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // 37n sibling (background-color / line-height / counter-* / content /
        // string-set / position / text-align) の wire-through pattern を踏襲
        // (原則 1 前例主義)。
        use crate::property::BoxSizing;
        let cv = cascade_doc("", "p", Some("box-sizing: border-box"));
        assert_eq!(cv.box_sizing, BoxSizing::BorderBox);
    }

    // ── font-weight keyword + inheritance (CSS Fonts 4 §2.2、raikiri-spike-0vv.16) ──

    #[test]
    fn font_weight_keyword_bold_wired_through_cascade_from_inline_style() {
        // <p style="font-weight: bold"> → ComputedValues.font_weight = 700。
        // parser Ident arm → PropertyValue::FontWeight(Absolute(700)) → apply_value →
        // ComputedValues の end-to-end 疎通 smoke (0vv.8 / 0vv.9 pattern を踏襲)。
        let cv = cascade_doc("", "p", Some("font-weight: bold"));
        assert_eq!(cv.font_weight, 700);
    }

    #[test]
    fn font_weight_keyword_normal_wired_through_cascade_from_inline_style() {
        // <p style="font-weight: normal"> → ComputedValues.font_weight = 400。
        let cv = cascade_doc("", "p", Some("font-weight: normal"));
        assert_eq!(cv.font_weight, 400);
    }

    #[test]
    fn font_weight_is_inherited_child_carries_parent_bold() {
        // CSS Fonts 4 §2.2 "Inheritance: Yes"。<p style="font-weight: bold"> の
        // 子 <span> は自身 rule 無しでも parent の 700 を継承する。
        // Verification #7 (bd 0vv.16): parent bold + child 未指定 = child 700。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bold"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700);
        assert_eq!(
            r.computed[span].font_weight, 700,
            "font-weight must be inherited (CSS Fonts 4 §2.2 Yes) — \
             parent bold keyword → child inherits 700"
        );
    }

    // ── font-weight bolder / lighter (CSS Fonts 4 §2.2、raikiri-spike-17s8) ──

    /// `<p style="font-weight: {parent}"><span style="font-weight: {child}">` を
    /// cascade して span の computed font-weight を返す。
    ///
    /// **親の computed value** を経由することが本 helper の主眼 — child は
    /// literal な spec 値ではなく、親が cascade を通して確定させた weight に
    /// 対して relative resolution される (bd 17s8 Verification #7)。
    fn relative_weight_through_cascade(parent_decl: &str, child_decl: &str) -> u16 {
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
        // 検証する。unit 関数を叩くことで cascade harness に依存せず表の
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
        assert_eq!(bolder(1), 400);
        assert_eq!(bolder(99), 400);
        assert_eq!(lighter(1), 1);
        assert_eq!(lighter(99), 99);
        // row 2: 100 <= w < 350
        assert_eq!(bolder(100), 400);
        assert_eq!(bolder(349), 400);
        assert_eq!(lighter(100), 100);
        assert_eq!(lighter(349), 100);
        // row 3: 350 <= w < 550
        assert_eq!(bolder(350), 700);
        assert_eq!(bolder(549), 700);
        assert_eq!(lighter(350), 100);
        assert_eq!(lighter(549), 100);
        // row 4: 550 <= w < 750
        assert_eq!(bolder(550), 900);
        assert_eq!(bolder(749), 900);
        assert_eq!(lighter(550), 400);
        assert_eq!(lighter(749), 400);
        // row 5: 750 <= w < 900
        assert_eq!(bolder(750), 900);
        assert_eq!(bolder(899), 900);
        assert_eq!(lighter(750), 700);
        assert_eq!(lighter(899), 700);
        // row 6: 900 <= w (bolder = no change)
        assert_eq!(bolder(900), 900);
        assert_eq!(bolder(1000), 1000);
        assert_eq!(lighter(900), 700);
        assert_eq!(lighter(1000), 700);
    }

    #[test]
    fn font_weight_table_no_change_rows_are_not_clamps() {
        // 表の両端 2 行は "no change" であって clamp ではない。算術近似
        // (`min(w + 300, 900)` / `max(w - 300, 100)`) を書くとここが壊れる。
        // この 2 行は raikiri-spike-5iy が range を [1,1000] に広げて初めて
        // author から到達可能になったため、bundle 固有の regression guard。
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Bolder, 1000),
            1000,
            "900 <= w row is no-change: bolder(1000) must stay 1000, not clamp to 900"
        );
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Lighter, 50),
            50,
            "w < 100 row is no-change: lighter(50) must stay 50, not rise to 100"
        );
    }

    #[test]
    fn font_weight_absolute_ignores_inherited_weight() {
        // `<font-weight-absolute>` は継承値と無関係にそのまま computed になる。
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Absolute(250), 900),
            250
        );
    }

    #[test]
    fn font_weight_bolder_wired_through_cascade_from_parent_computed() {
        // bd 17s8 Verification #3 / #4 / #5。親の **computed** weight に対して
        // resolve される (parse → PropertyValue::FontWeight(Bolder) →
        // apply_value → ComputedValues の end-to-end 疎通)。
        assert_eq!(
            relative_weight_through_cascade("font-weight: 400", "font-weight: bolder"),
            700
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 700", "font-weight: bolder"),
            900
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 900", "font-weight: bolder"),
            900,
            "900 <= w row: bolder is a no-op at 900"
        );
    }

    #[test]
    fn font_weight_lighter_wired_through_cascade_from_parent_computed() {
        // bd 17s8 Verification #6。
        assert_eq!(
            relative_weight_through_cascade("font-weight: 100", "font-weight: lighter"),
            100,
            "100 <= w < 350 row: lighter(100) = 100"
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 700", "font-weight: lighter"),
            400
        );
    }

    #[test]
    fn font_weight_relative_resolves_against_computed_not_literal_parent_value() {
        // bd 17s8 Verification #7 の核心。親の declaration は `bold` keyword
        // (literal な spec 値は "bold" であって数値ではない) だが、resolution は
        // 親の **computed** 700 に対して行われる → bolder(700) = 900。
        assert_eq!(
            relative_weight_through_cascade("font-weight: bold", "font-weight: bolder"),
            900,
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
            r.computed[p].font_weight, 700,
            "root-level bolder resolves against the initial 400"
        );
        assert_eq!(
            r.computed[span].font_weight, 900,
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
        assert_eq!(r.computed[span].font_weight, 700);
    }

    #[test]
    fn font_weight_full_range_wired_through_cascade() {
        // raikiri-spike-5iy: spec range [1,1000] の両端が cascade まで届く。
        assert_eq!(cascade_doc("", "p", Some("font-weight: 1")).font_weight, 1);
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1000")).font_weight,
            1000
        );
        // fractional は parse 段で round-half-away-from-zero 済み。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 100.5")).font_weight,
            101
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

    // ── Cascade memory DoS regression (raikiri-spike-d9y.1、SEC HIGH) ──
    //
    // Codex Cloud Security finding: 従来 `PropertyValue::Content(Vec<ContentComponent>)` /
    // `ComputedValues.content: Vec<ContentComponent>` は cascade 段の `decl.value.clone()`、
    // `apply_winners` の drain の `value.clone()`、`resolve_inheritance` の stack push + write と
    // 段階ごとに deep-clone を経由し、`* { content: "<large>" }` × N element で
    // O(N × M) 相当の heap 消費を招いていた。d9y.1 で outer `Vec` を
    // `Arc<Vec<ContentComponent>>` に wrap、全 clone 経路が Arc bump に落ちた。
    //
    // 実 heap 計測は環境依存 (allocator hook が必要) のため、behavioral proxy として
    // `Arc::ptr_eq` で「複数 element が同 rule から同一 underlying `Vec` を共有」を
    // 確認する。Regression 時 (deep clone に戻る) はここが false となり test fail する。

    /// `* { content: "<literal>" }` × N element の cascade で、matching 全 element
    /// の `ComputedValues.content` Arc は **同 underlying Vec** を指す
    /// (`Arc::ptr_eq` = true)。deep-clone regression の behavioral canary。
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
             (raikiri-spike-d9y.1 SEC HIGH DoS regression)"
        );
    }

    /// `string-set` も content と同じ cascade path を辿るため、同種 Arc 共有が
    /// 成立している必要がある (d9y.1 で `PropertyValue::StringSet` も Arc wrap)。
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
            "cascade must Arc-share string_set across universal-selector matches \
             (raikiri-spike-d9y.1)"
        );
    }

    // ── Cascade memory DoS regression (raikiri-spike-d9y.2、SEC HIGH) ──
    //
    // Codex Cloud Security finding (finding hash 12128875): `counter-reset` /
    // `counter-increment` / `counter-set` は d9y.1 の Content/StringSet と同じ
    // 3 段 clone 経路 (`decl.value.clone`、`apply_winners` の drain の `value.clone`、
    // `resolve_inheritance` の stack push + write) を辿るため
    // `PropertyValue::Counter*(Vec<..>)` × universal selector × N element で
    // O(N × M) 相当の heap 消費を招いていた。d9y.2 で全 3 property の outer
    // `Vec` を `Arc<Vec<(SmolStr, i32)>>` に wrap、clone 経路が Arc bump に
    // 落ちた (asymptotic は O(N + M))。short-circuit (child stack entry で
    // counter-* を skip) は **意図的に採用せず** — d9y.1 の Content/StringSet も
    // 同じ non-inherited Arc field でありながら short-circuit していないため、
    // counter-* のみ特別扱いすると仕上げが非対称になる。Arc wrap 単独で DoS は
    // 塞がる (stack 上に転がるのは Arc bump 1 個ずつだけで、直後の
    // `inherit_from` で empty slot に落ちる)。

    /// `* { counter-reset: <list> }` × N element の cascade で、matching 全
    /// element の `ComputedValues.counter_reset` Arc は **同 underlying Vec** を
    /// 指す (`Arc::ptr_eq` = true)。deep-clone regression の behavioral canary。
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
             (raikiri-spike-d9y.2 SEC HIGH DoS regression)"
        );
    }

    /// `counter-increment` も counter-reset と同じ cascade path を辿るため、
    /// 同種 Arc 共有が成立している必要がある (d9y.2)。
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
            "cascade must Arc-share counter_increment across universal-selector matches \
             (raikiri-spike-d9y.2)"
        );
    }

    /// `counter-set` も同じ cascade path を辿るため Arc 共有が必要 (d9y.2)。
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
            "cascade must Arc-share counter_set across universal-selector matches \
             (raikiri-spike-d9y.2)"
        );
    }

    /// counter-* は non-inherited (CSS Lists 3 §3) — 親 element に counter 値が
    /// あっても child は inherit_from で shared empty Arc slot に落ちる。この
    /// pin が「Arc wrap 単独 (short-circuit 無し) でも child stack entry の
    /// parent Arc bump が即 empty slot に置換される」ことを保証する。d9y.2
    /// short-circuit 不採用の正当化 assertion。
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
        // どちらも d9y.2 の memory 目標を破る。
        let shared = crate::property::empty_counter_entries();
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[child].counter_reset, &shared),
            "child counter_reset must point to shared empty Arc slot \
             (raikiri-spike-d9y.2 non-inherited short-circuit-equivalent canary)"
        );
    }

    /// Empty (initial / inherit_from) の content/string_set も **shared Arc slot**
    /// を再利用する — cascade fix の副作用で「per-node empty Arc allocation
    /// regression」に陥っていないことを pin (advisor calibration)。
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
             allocation regression detected (raikiri-spike-d9y.1 side-effect canary)"
        );
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].string_set, &r.computed[p2].string_set),
            "empty string_set must reuse shared Arc slot (raikiri-spike-d9y.1 side-effect canary)"
        );
    }

    // ── padding wire-through (CSS Box 3 §6.1 + §6.2、raikiri-spike-0vv.6) ──
    //
    // Verification #7 (cascade wire-through + non-inheritance):
    // <div style="padding: 10px 5%"> の cascade 結果が populate、initial value 0
    // が inheritance walk で child に伝播しない (padding は non-inherited)。

    #[test]
    fn padding_shorthand_wired_through_cascade_from_inline_style() {
        // Verification #7: `padding: 10px 5%` → 2-value form expansion で
        // top/bottom=10px, left/right=5% を pin。counter-* / content / string_set
        // wire-through pattern を踏襲 (s85 / m5.1 / m5.3、raikiri-spike-0vv.6)。
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
        // 37n sibling: string_set / content / display / counter-* / running_templates
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
        // spec §6.1 negative reject の end-to-end smoke: cascade まで負値が
        // 到達せず、initial (0) が残る。property.rs test は parse_value 単体
        // の drop、本 test は rule.rs → cascade の一貫 drop を pin。
        use crate::property::Sides;
        let cv = cascade_doc("", "div", Some("padding-top: -5px"));
        // 負値 → declaration drop → padding は cascade 未 override → initial 0 が残る。
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    #[test]
    fn padding_shorthand_then_longhand_longhand_wins() {
        // CSS Cascading L5 §6: shorthand は parse-time で longhand に expand
        // してから cascade する。`padding: 10px; padding-top: 5px;` →
        // top=5, others=10 (source-order-independent、spec-correct)。
        // (margin 0vv.5 で実装済みの parse-time expansion model に migtate:
        // raikiri-spike-5nc)
        let cv = cascade_doc("", "div", Some("padding: 10px; padding-top: 5px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    #[test]
    fn padding_longhand_then_shorthand_shorthand_wins() {
        // spec §6.4.4 の後方 wins を逆順で pin: `padding-top: 5px; padding: 10px;`
        // → 全 side = 10px (後段 shorthand が top も含めて上書き)。
        // (raikiri-spike-5nc)
        let cv = cascade_doc("", "div", Some("padding-top: 5px; padding: 10px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    // ── margin longhand + shorthand cascade (CSS Box 3 §3.1/§3.2、raikiri-spike-0vv.5) ──

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
        // spec (CSS Cascading L4 §6.4.4): 同一 declaration block 内で shorthand
        // + longhand が declared された場合、後方 declaration が同 rank/spec/order
        // で勝つ。`margin: 0px; margin-top: 10px;` → top=10, others=0。
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
        // spec §6.4.4 の後方 wins を逆順で pin: `margin-top: 10px; margin: 0px;`
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
        // 37n sibling: display / string_set / content non-inherited と同 shape。
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
        // で受理されることを pin (parser 側 pin `margin_side_accepts_negative_length`
        // と complementary、下流 layout 側で negative 意味付け)。
        let cv = cascade_doc("", "div", Some("margin-top: -5px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(-5.0));
    }

    // ── height wire-through (CSS Sizing 3 §3.1.1、raikiri-spike-0vv.11) ──

    #[test]
    fn height_wired_through_cascade_from_inline_style() {
        // <div style="height: 100px"> → ComputedValues.height に
        // LengthOrAuto::Length(Length::Px(100)) が届く。parser →
        // PropertyValue::Height → apply_value → ComputedValues の end-to-end
        // 疎通 smoke (0vv.5 margin / 0vv.6 padding wire-through pattern を踏襲)。
        let cv = cascade_doc("", "div", Some("height: 100px"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn height_auto_wired_through_cascade() {
        // `height: auto` は spec initial (§3.1.1) だが cascade winner として
        // declaration が到達した場合の受理 pattern を明示 pin
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
        // height = initial (`LengthOrAuto::Auto`)。37n sibling: margin / padding
        // / display / string_set / content non-inherited と同 shape
        // (raikiri-spike-0vv.11)。
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
        // まま。parser 側 pin (`height_rejects_negative_length`) と complementary
        // な end-to-end 挙動を確認。
        let cv = cascade_doc("", "div", Some("height: -10px"));
        assert_eq!(
            cv.height,
            ComputedLengthPercentageOrAuto::Auto,
            "negative height declaration must be dropped; height stays at initial"
        );
    }

    #[test]
    fn apply_value_direct_margin_shorthand_safety_net() {
        // `apply_value` の `PropertyValue::Margin(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 4 longhand に展開する) だが、
        // regression / bypass 経路の safety net として `target.margin = sides` の
        // atomic 上書きを持つ。本 test は arm を直接叩いて `unreachable!` 化 or
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

    // ── border longhand + shorthand cascade (raikiri-spike-0vv.12) ──

    #[test]
    fn border_shorthand_then_longhand_later_longhand_wins() {
        // spec (CSS Cascading L5 §6.4.4): 同一 declaration block 内で shorthand
        // + longhand が declared された場合、後方 declaration が同 rank/spec/order
        // で勝つ。`border: 1px solid red; border-top-width: 10px;` →
        // top.width=10、他 side の width=1、top.style=Solid、top.color=red 保持。
        //
        // 本 test は本 architecture の load-bearing case (advisor calibration):
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
        // raikiri-spike-0vv.17: border.color は `BorderColor` enum、shorthand
        // 由来の author-specified red は `Resolved` variant で cascade に届く。
        assert_eq!(cv.border.top.color, BorderColor::Resolved(red));
        assert_eq!(cv.border.right.style, BorderStyle::Solid);
        assert_eq!(cv.border.left.color, BorderColor::Resolved(red));
    }

    #[test]
    fn border_longhand_then_shorthand_later_shorthand_wins() {
        // spec §6.4.4 の後方 wins を逆順で pin: `border-top-width: 10px; border:
        // 1px solid red;` → top.width も 1px (後段 shorthand が top も含めて
        // 上書き)。expand_shorthand_into の 12 longhand 展開が source_order を
        // 保持したまま cascade に届き、後段が per-side / per-sub-property
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
        // CSS Backgrounds 3 §5 "Inherited: no"。<div style="border: 5px solid red">
        // の子 <span> は自身 rule 無しで border = initial (medium / none /
        // currentcolor)。37n sibling: margin / padding non-inherited test
        // を踏襲。raikiri-spike-0vv.17: color は `BorderColor` enum で保持。
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
    fn apply_value_direct_border_shorthand_safety_net() {
        // `apply_value` の `PropertyValue::Border(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 12 longhand に展開する) だが、
        // regression / bypass 経路の safety net として `target.border = sides` の
        // atomic 上書きを持つ。本 test は arm を直接叩いて `unreachable!` 化 or
        // 空 arm regression を捕捉する canary (margin safety net と対称)。
        let mut cv = SpecifiedValues::initial();
        // raikiri-spike-0vv.17: `BorderColor::CurrentColor` を明示 fixture 化
        // (safety net arm は payload の shape を保持することを pin する)。
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
    fn width_length_end_to_end() {
        // Verification #7: `div { width: 100px }` が `ComputedValues.width` に
        // Length(Px(100)) として届く。parser → PropertyValue::Width → apply_value
        // → ComputedValues の end-to-end 疎通 smoke (sibling padding/margin と
        // 同 pattern)。raikiri-spike-0vv.10。
        let cv = cascade_doc("", "div", Some("width: 100px"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn width_auto_end_to_end() {
        // `width: auto` は cascade winner として apply_value で `Auto` に固定される。
        // `inherit_from` initial も Auto なので identity になるが、cascade path が
        // 実際に通っていることを pin (silent no-op regression 検知)。
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
        // の end-to-end pin (sibling `inherit_from_leaves_*_at_initial` computed
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
    fn cascade_with_ua_deterministic_across_10_runs() {
        // determinism regression (spec §M1 acceptance criteria)
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red }");
        let p = doc.push_element(0, "p", Some("font-size: 20px"));
        doc.push_element(p, "span", None);

        let mut tree = build_rule_tree(&doc);
        tree.add_stylesheet(
            "p { display: block } span { display: inline }",
            Origin::UserAgent,
        );

        let baseline = cascade(&doc, &tree).unwrap();
        for _ in 0..9 {
            let run = cascade(&doc, &tree).unwrap();
            assert_eq!(run.computed.len(), baseline.computed.len());
            for i in 0..run.computed.len() {
                assert_eq!(run.computed[i], baseline.computed[i], "differ at node {i}");
            }
        }
    }

    // ── post-parse shorthand injection (bd raikiri-spike-nqkj) ──────────────
    //
    // `RuleTree.style_rules` / `StyleRule.declarations` / `Declaration.value`
    // は `pub` field で、`raikiri` umbrella crate が `RuleTree` を re-export して
    // いる。したがって Consumer は `add_stylesheet` の**後**に declaration を
    // shorthand variant へ書き戻せる — `crate::rule::parse_declaration_block` の
    // parse-time 展開はこの経路を守らない (`declaration_block_never_emits_
    // shorthand_keys` は parse 出口しか見ない)。
    //
    // 守るべき spec は 2 条:
    //
    // - CSS Cascading L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand>
    //   "A shorthand property sets all of its longhand sub-properties, exactly
    //   as if expanded in place."
    // - CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
    //   "Order of Appearance: … the last declaration in document order wins."
    //
    // すなわち勝者は **declaration の並び順**で決まる。`PropertyKey` discriminant
    // 順 (shorthand が longhand より後) に任せると、shorthand が先に書かれた
    // 場合に spec と逆になる。`collect_cascaded` が candidate を積む直前に
    // `expand_shorthand_into` を通すことでこれを塞ぐ。

    /// nqkj の repro を機械的に再現する helper。
    ///
    /// `css` を `RuleTree::add_stylesheet` で parse したあと、Consumer と同じ
    /// 手つきで `style_rules[0].declarations[idx].value` を `injected` に差し替え、
    /// `<div>` 1 個の document に cascade して computed value を返す。
    ///
    /// `StyleRule` を直接 literal 構築しないのが要点 — `#[non_exhaustive]` は
    /// crate 内構築を妨げないので、literal だと **report された経路とは別の
    /// 経路**を test してしまう。
    ///
    /// ⚠️ 差し替えるのは `.value` **だけ**なので、`css` の `idx` 番目の
    /// declaration は 2 役を持つ:
    ///
    /// 1. **値は捨てられる** — 単に「その位置に declaration を 1 個作る」ための
    ///    placeholder (本 module の呼び出しでは `margin-left: 99px` 等、
    ///    以降の assert に一切現れない値を置いている)。
    /// 2. **`important` flag は生き残り、injected shorthand に載る** — 展開時に各
    ///    longhand へ copy される。これを利用して「注入 shorthand は normal」を
    ///    作っているのが
    ///    `post_parse_important_longhand_survives_later_normal_shorthand`
    ///    である (placeholder に `!important` を付けると逆になる)。
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

    /// 4 side が全て互いに異なり、かつ initial (0) とも異なる margin fixture。
    /// `Sides::all` だと「1 side しか展開していない」実装と「4 side 展開した」
    /// 実装が区別できず、0 を使うと initial と区別できない。
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
        //   [0] Margin(1,2,3,4)   ← 注入
        //   [1] MarginTop(10px)
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
        //   [0] MarginTop(10px)
        //   [1] Margin(1,2,3,4)   ← 注入
        // spec §6.1 → 全 side が shorthand 由来 = 1/2/3/4。
        //
        // 本方向は展開しない実装でも偶然一致するが、fix が「shorthand を
        // 常に負けさせる」誤った非対称化になっていないことを pin する。
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
        // margin と同じ形を padding family でも pin (展開 arm が family ごとに
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
        // 落ちていないことを pin する (style は computed width の gating にも
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
        // 展開時の `!important` copy (spec §3 "Declaring a shorthand property
        // to be !important is equivalent to declaring all of its sub-properties
        // to be !important.") が cascade 入口の展開でも保たれること。
        //
        // 注入した shorthand は `!important` を継承する ([0] は `!important`
        // 付きで parse される) ので、後方の normal longhand には**負けない**。
        //
        // ⚠️ **本 test は展開の有無を区別しない** (§8.2 spec lens が hunk revert
        // で実測)。展開しない実装でも `PropertyKey` 宣言順 (`Margin` が
        // `MarginTop..MarginLeft` より後) のせいで `Margin` slot が最後に
        // atomic 適用され、偶然同じ 1/2/3/4 になるため。`!important` 方向で
        // 展開の有無を実際に区別するのは
        // `post_parse_important_longhand_survives_later_normal_shorthand` で
        // あり、本 test は「展開が important flag を落としていない」だけを見る
        // 弱い guard である (`..._after_longhand_lets_shorthand_win` と同種)。
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
        //   [0] MarginTop(10px) !important
        //   [1] Margin(1,2,3,4)  normal   ← 注入 (`important` は false のまま)
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
}
