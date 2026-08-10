//! CSS cascade + inheritance walk (M1.4)。
//!
//! 2 phase:
//! 1. per-node "cascaded values" 決定 — matching rule + inline style の候補集合から
//!    specificity + !important + source order で winner を選択
//! 2. inheritance walk — top-down DFS で親の computed value を継承 + 自 node の
//!    cascaded value で override
//!
//! # inheritance walk 内部の 4 段階 (bd decision raikiri-spike-082k、
//! # phase 2.5 は bd raikiri-spike-vxha で追加)
//!
//! 上記 phase 2 の per-node 処理は、さらに 4 段に分かれる (順に phase 1 /
//! 2 / 2.5 / 3 と呼ぶ):
//!
//! - **phase 1: winner の staging** — 親の [`ComputedValues`] から
//!   [`SpecifiedValues`] を seed し、その node の全 winner を `apply_value` で
//!   適用する。この段では length は specified 表現のまま。
//! - **phase 2: font-size の絶対化** — **親の** computed font-size 基準。
//! - **phase 2.5: line-height の絶対化** — 自 node の (今確定した) font-size
//!   基準。`lh`/`rlh` の自己参照基準の非対称は
//!   [`crate::resolve::resolve_line_height`] doc が canonical。
//! - **phase 3: 残り全 length の絶対化** — **自 node の** computed font-size /
//!   line-height 基準。
//!
//! 2 / 2.5 / 3 は [`SpecifiedValues::finalize`] に閉じている。分離が必要な理由は
//! [`crate::specified`] の module doc を参照 (`padding: 2em` の基準となる
//! `font-size` はその node の**全** winner を適用し終えるまで確定しないため、
//! winner 適用の途中で絶対化することはできない)。

use std::collections::HashMap;
use std::ops::Range;

use cssparser::{Parser, ParserInput};
use selectors::attr::{CaseSensitivity, ParsedCaseSensitivity};
use selectors::parser::{Selector, SelectorList};

use crate::RaikiriSelectorImpl;
use crate::computed::{ComputedValues, RunningTemplate};
use crate::error::CascadeError;
use crate::property::{
    FontWeightValue, Length, LengthOrAuto, PositionValue, PropertyValue, RelativeFontSize,
    resolve_text_align_match_parent,
};
use crate::resolve::{ComputedLength, ResolveContext, used_line_height_length};
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
    let mut cascaded = CascadedArena::new();

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

/// inline style の specificity。CSS Cascading L4 §6.1 "Cascade Sorting Order"
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の Specificity 段 verbatim:
/// "declarations that do not belong to a style rule (such as the contents of a
/// style attribute) are considered to have a specificity higher than any
/// selector." (`(1, 0, 0, 0)` は CSS 2.1 §6.4.3 の旧表現であり L4 の規定ではない)。
/// selectors crate は 32-bit packed で `id << 20 | class << 10 | element` を使う。
/// `1 << 30` はその packed 空間のどの selector 由来 specificity よりも大きいので、
/// 上記 "higher than any selector" を満たす。**この margin はちょうど 1** であり
/// upstream が packing 幅を広げると反転しうる不変条件 — pin は
/// `tests::inline_specificity_exceeds_max_reachable_packed_specificity` を参照
/// (bd raikiri-spike-nvhy)。
const INLINE_SPECIFICITY: Specificity = 1 << 30;
/// inline style の source_order — 全 stylesheet rule より後 (最終出現扱い)。
const INLINE_SOURCE_ORDER: u32 = u32::MAX;

/// HTML presentational hint (`<img width>` / `<img height>`,
/// [`push_img_dimension_hints`]) の specificity。HTML LS §15.2 "The CSS user
/// agent style sheet and presentational hints"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#presentational-hints>)
/// がこの種の hint を "author-level **zero-specificity** presentational
/// hints" と呼ぶ — 0 はその verbatim な表現 (CSS Cascading L5 §6.5
/// <https://drafts.csswg.org/css-cascade-5/#preshint> は origin 選択の
/// 枠組みを定義するのみで "specificity" という語自体は使っていない —
/// 引用元を混同しないよう分離。origin 選択の根拠は
/// [`push_img_dimension_hints`] doc の "Cascade origin" 節参照)。
const PRESENTATIONAL_HINT_SPECIFICITY: Specificity = 0;
/// 同 hint の source_order。`0` — 「他候補と衝突しない値」ではなく、**実
/// stylesheet の最初の rule と数値上 tie し得る**ことを承知の上で選んだ値
/// ([`crate::ruletree::RuleTree::add_stylesheet`] は空 `RuleTree` への
/// 最初の rule に `source_order = 0` を採番する)。tie した場合の決着は
/// push 順序依存になる — [`collect_cascaded`] がこの hint を stylesheet
/// rule matching / inline style より先に push する理由、および
/// [`push_img_dimension_hints`] doc の "Cascade origin" 節 2. 参照。
const PRESENTATIONAL_HINT_SOURCE_ORDER: u32 = 0;

/// 1 candidate declaration = `(value, important, origin, specificity, source_order)`。
/// `collect_cascaded` が populate、`pick_winners` が rank 化して winner を選ぶ
/// (raikiri-spike-m1.22 で `Origin` を追加、clippy::type_complexity 回避のため alias 化)。
type CascadedDecl = (PropertyValue, bool, Origin, Specificity, u32);

/// [`collect_cascaded`] の出力 — 全 node 分の candidate を単一 flat `Vec` に
/// 積み、node ごとの部分区間を [`Range`] で引く (bd raikiri-spike-gerj、
/// raikiri-spike-8kn8 の follow-up)。
///
/// # 何を置換したか
///
/// 旧実装は `HashMap<StyleNodeId, Vec<CascadedDecl>>` — per-node に `Vec` を
/// 1 本ずつ確保していた。n=1000 node の cascade で **collect_cascaded 単体
/// 4,030 allocs / 2,439,940 bytes** (bd raikiri-spike-gerj 実測、着手時
/// 再計測。8kn8 起票時の「collect_cascaded 他 rest」バケツは
/// `resolve_inheritance` の clone chain と合算されていたため、それとは別数値)。
/// ただしこの 4,030 のうち **1,020 allocs / 64,744 bytes は本 struct が
/// 触れていない `dom.child_ids(id).collect()` 行**に由来していた (同じ doc で
/// その行だけを単独実行して確認、gerj 当時は bd raikiri-spike-75ch の管轄で
/// 本 struct の対象外)。この残差は 75ch が [`collect_cascaded`] /
/// [`resolve_inheritance`] 双方の呼び出し箇所を「捨て `Vec` へ `collect` して
/// `rev()`」から「`stack` へ直接 `extend` してから追加分だけ in-place
/// `reverse()`」に書き換えて解消済み — 中間 allocation はもう存在しない
/// (同じ形の第 3 の call site だった `crates/raikiri-style/src/ruletree.rs` の
/// `walk_style_elements` も bd raikiri-spike-o53w が同じ技法で解消済み、本
/// module の対象外)。per-node `Vec` の growth chain 自体が担っていたのは残り
/// **3,010 allocs / 2,375,196 bytes** — push のたび geometric に再確保する
/// その growth chain が丸ごと allocation cost だった。単一 arena にすると
/// growth chain は文書全体で 1 本になり (n=1000 で 23 allocs まで低下、
/// -99.2%)、chain 長は `O(log 総 candidate 数)` に潰れる。
///
/// # なぜ struct で wrap するか (bare `(Vec<_>, HashMap<_, Range<usize>>)` にしないか)
///
/// [`pick_winners`] の `winner.idx` は「渡された **その** slice 内の位置」で
/// あり、bd raikiri-spike-8kn8 のドキュメントが警告する通り **範囲外にならず
/// 静かに別 node の宣言を読む** 経路がある。arena 化で新たに生まれる同型の
/// 危険は「[`candidates`](Self::candidates) を経由せず、`decls` 全体や
/// `decls[range.start..]` のような**部分的に間違ったスライス**を
/// [`apply_winners`] に渡してしまう」こと — この場合も範囲外にはならず、
/// 別 node の候補を静かに拾う。fields を private にして
/// [`candidates`](Self::candidates) だけを公開することで、呼び出し側は
/// 「この node 自身の区間ちょうど」以外のスライスを **作れない**。
///
/// `pub(crate)` は [`resolve_inheritance`] 自身が `pub(crate)` (他 module の
/// doc からの intra-doc link のため) であることに追随するだけで、他 module
/// から構築/操作されることは想定していない — 構築は [`cascade`] が行い、
/// 内容の書き込みは [`collect_cascaded`] に閉じている (いずれも本 module)。
pub(crate) struct CascadedArena {
    /// 全 node の candidate を document 内 visit 順で連結した flat 領域。
    decls: Vec<CascadedDecl>,
    /// node ごとの `decls` 内部分区間。空 (no candidate) の node はここに
    /// entry を持たない — 旧実装の `if !per_node.is_empty() { out.insert(..) }`
    /// と同じ「無ければ握らない」契約。
    ranges: HashMap<StyleNodeId, Range<usize>>,
}

impl CascadedArena {
    fn new() -> Self {
        Self {
            decls: Vec::new(),
            ranges: HashMap::new(),
        }
    }

    /// `id` 自身の candidate 一覧 — [`pick_winners`] にそのまま渡せる
    /// **自 node 専用**のスライス。
    ///
    /// 返す slice は常に `&self.decls[range]` で `range` は `id` のために
    /// `collect_cascaded` が積んだ区間ちょうど — 呼び出し側がこれ以外の形の
    /// slice (全体、あるいは `range.start` だけずらしたもの) を組み立てる経路は
    /// 本 struct に存在しない。
    fn candidates(&self, id: StyleNodeId) -> Option<&[CascadedDecl]> {
        self.ranges.get(&id).map(|range| &self.decls[range.clone()])
    }
}

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
/// 高いほど勝つ。CSS Cascading L4 §6.1 "Cascade Sorting Order"
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の Origin and Importance
/// 段を表現する (origin の定義は §6.2
/// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>、`!important` に
/// よる反転は §6.3 <https://www.w3.org/TR/css-cascade-4/#importance>):
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
///
/// # flat arena への書き込み (bd raikiri-spike-gerj)
///
/// 1 node 分の candidate は `out.decls` に**連続して**積まれる —
/// stylesheet rule matching (rule/declaration の source order) → inline
/// style の順に push し、両方終わったところで `start..out.decls.len()` を
/// その node の区間として登録する。次の node の処理が始まるまで他の push が
/// 割り込まないことが「区間が連続」の根拠であり、
/// [`CascadedArena::candidates`] が返す slice の index が
/// [`pick_winners`]/[`apply_winners`] にとって**その node 自身の**
/// `candidates` 内 index であり続ける前提そのもの (raikiri-spike-8kn8 の
/// 「global index space を渡すと壊れる」警告を参照)。
fn collect_cascaded<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut CascadedArena,
) {
    let mut stack: Vec<StyleNodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            // silent bug fix: 従来 template 内 element にも rule matching が走り
            // arena (旧実装では per-node Vec<CascadedDecl>) が waste で膨らんで
            // いた。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let start = out.decls.len();
                // HTML presentational hints (bd raikiri-spike-5z86.7). MUST
                // be pushed before stylesheet-rule matching / inline style
                // below, for this same element: the hint's (origin,
                // specificity, source_order) = (Author, 0, 0) can exactly
                // tie a real Author declaration for the same property (a
                // zero-specificity selector that is the first rule in its
                // stylesheet — `push_img_dimension_hints` doc's "Cascade
                // origin" §2 has the full derivation). `beats`'s `>=`
                // resolves an exact tie in favor of whichever candidate
                // `pick_winners` scans *later*; pushing the hint first
                // guarantees it loses that tie to any same-priority real
                // declaration, matching "hint behaves as if positioned
                // before all real author declarations".
                push_img_dimension_hints(&elem, &mut out.decls);
                // stylesheet rule matching
                for rule in &rule_tree.style_rules {
                    if let Some(spec) = match_simple_selectors(&rule.selectors, &elem) {
                        for decl in &rule.declarations {
                            // shorthand を longhand に展開してから candidate に
                            // 積む (parse 出口の展開だけでは `RuleTree` の
                            // post-parse mutation 経路を守れないため —
                            // bd raikiri-spike-nqkj)。rationale は
                            // `crate::rule::expand_shorthand_into` doc に集約。
                            expand_shorthand_into(decl, |d| {
                                out.decls.push((
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
                        out.decls.push((
                            decl.value,
                            decl.important,
                            Origin::Author,
                            INLINE_SPECIFICITY,
                            INLINE_SOURCE_ORDER,
                        ));
                    }
                }
                let end = out.decls.len();
                if end > start {
                    out.ranges.insert(id, start..end);
                }
            }
            // stack は LIFO なので document order で push するため reverse。
            // `child_ids` イテレータを直接 `stack` へ `extend` し、今回追加した
            // 末尾スライスだけを in-place `reverse()` する — 都度捨てる中間
            // `Vec` を経由しない (bd raikiri-spike-75ch)。`stack` 自体の
            // capacity growth は元の `for .. { stack.push(..) }` と同じ
            // amortized pattern のままで、ここで削れるのは「今回だけの捨て
            // Vec」1 本分のみ。
            //
            // なぜ document order を保つか: 本関数冒頭のコメント (上記
            // 「per-node の処理は他の node の状態に依存しないため訪問順は
            // 無関係」) の通り、collect_cascaded 自体の正しさは訪問順に依存
            // しない。ここで document order を維持しているのは純粋に
            // refactor 前との**挙動の完全一致**のためで、新たな正しさ上の
            // 要請ではない。
            let start = stack.len();
            stack.extend(dom.child_ids(id));
            stack[start..].reverse();
        }
    }
}

/// `elem` と selector list を突き合わせる simple-selector matcher (single
/// element のみ — combinator を要する tree-walk は非対応、bd raikiri-spike-flln.1)。
///
/// Returns: matching した selector の最大 specificity。1 つも match しなければ None。
/// - `Component::LocalName(name)` — `elem.tag_name()` と eq_ignore_ascii_case で判定
/// - `Component::ExplicitUniversalType` — 常に match
/// - `Component::ID` — `elem.id()` と厳密一致 (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#id-selectors>、HTML の `id` は
///   case-sensitive)
/// - `Component::Class` — `elem.has_class()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#class-html>、
///   [`StyleElement::has_class`] の doc 通り HTML-spec ASCII whitespace split
///   + token 単位 case-sensitive 比較)
/// - `Component::AttributeInNoNamespaceExists` / `Component::AttributeInNoNamespace`
///   — `elem.attr()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#attribute-selectors>)。存在チェック
///   形態 (`[foo]`) の lookup key は element の namespace に応じて
///   `local_name` / `local_name_lower` を選ぶ (詳細は該当 match arm の
///   コメント)。値付き形態の case-sensitivity 解決は
///   [`resolve_case_sensitivity`] 参照
///
/// 他 component (combinator / pseudo-class / namespace 付き属性 selector = 常に
/// `Component::AttributeOther`、または非小文字 local name **かつ値付き**の
/// 属性 selector = 同じく `Component::AttributeOther` — 非小文字でも値なしの
/// 存在チェック形態は namespace 無指定なら `AttributeInNoNamespaceExists` の
/// まま、詳細は `ruletree.rs` `is_supported_selector_list` のコメント) は
/// `is_supported_selector_list` が rule tree 構築時点で drop 済のはずだが、
/// safety net として引き続き match fail する。
fn match_simple_selectors<E: StyleElement>(
    list: &SelectorList<RaikiriSelectorImpl>,
    elem: &E,
) -> Option<Specificity> {
    use selectors::parser::Component;

    let mut best: Option<Specificity> = None;
    for selector in list.slice() {
        let mut matches = true;
        for component in selector.iter_raw_match_order() {
            let component_matches = match component {
                Component::LocalName(local) => {
                    // local.name は Atom (raikiri-style::Atom)、tag_name 文字列と比較
                    elem.tag_name().eq_ignore_ascii_case(local.name.0.as_str())
                }
                Component::ExplicitUniversalType
                | Component::ExplicitAnyNamespace
                | Component::ExplicitNoNamespace
                | Component::DefaultNamespace(_) => {
                    // 常に match / namespace は m1.4 では常に true 扱い
                    true
                }
                // NB (bd raikiri-spike-tqwi): both arms below always compare
                // case-sensitively — CSS Selectors L4 requires ASCII-case-
                // insensitive id/class matching in quirks-mode documents
                // ("otherwise case-sensitive"), but raikiri-style has no
                // document-mode signal threaded through `StyleDom`/
                // `StyleElement` yet (`raikiri_traits::QuirksMode` stops at
                // the parse layer). Tracked by tqwi, not this task's scope.
                Component::ID(id) => elem.id() == Some(id.0.as_str()),
                Component::Class(class) => elem.has_class(class.0.as_str()),
                Component::AttributeInNoNamespaceExists {
                    local_name,
                    local_name_lower,
                } => {
                    // Which key to look up under `elem.attr()` depends on
                    // the *element's* namespace, unlike the with-value arm
                    // below (whose lowercase guarantee comes from the
                    // selector's own parse, not the element). HTML LS's
                    // "attributes on HTML elements in HTML documents are
                    // lowercased" scoping
                    // (https://html.spec.whatwg.org/multipage/semantics-other.html#case-sensitivity-of-selectors)
                    // only covers HTML-namespace elements — html5ever's
                    // tokenizer/tree-builder already lower-cases attribute
                    // names for those (confirmed via
                    // `crates/raikiri-html/src/sink.rs`'s `wire_side_tables`,
                    // which stores `a.name.local` verbatim with no extra
                    // lowercasing pass of its own), so `local_name_lower` is
                    // the correct — and actually-stored — key there. Foreign
                    // (SVG/MathML) elements are outside that HTML LS scope:
                    // html5ever's "adjust foreign attributes" step can
                    // restore specific attributes to their original mixed
                    // case (e.g. `viewBox`), and `wire_side_tables` stores
                    // whatever case html5ever produced, unmodified — so
                    // `local_name` (the selector's own, unmodified case) is
                    // the correct key for those.
                    let key = if elem.namespace_uri().is_none() {
                        local_name_lower
                    } else {
                        local_name
                    };
                    elem.attr(key.0.as_str()).is_some()
                }
                Component::AttributeInNoNamespace {
                    local_name,
                    operator,
                    value,
                    case_sensitivity,
                } => match elem.attr(local_name.0.as_str()) {
                    // `local_name` を直に使ってよい理由 (この with-value arm
                    // 限定 — 上の `AttributeInNoNamespaceExists` arm とは
                    // 対比的に element の namespace を問わない): `selectors`
                    // crate の parser (`AttributeInNoNamespace` を作る分岐) は
                    // *selector 自身の* local name が既に ASCII-lowercase な
                    // 場合にのみこの variant を選ぶ — 非小文字は
                    // `Component::AttributeOther` に回る
                    // (`is_supported_selector_list` が drop する)。この
                    // lowercase 保証は selector の parse 時点で決まり、
                    // どの element (HTML/foreign 問わず) に対して matching
                    // するかに依存しないため、上の Exists arm と違って
                    // namespace 分岐は不要。
                    Some(attr_value) => {
                        let case = resolve_case_sensitivity(*case_sensitivity, elem);
                        operator.eval_str(attr_value, value.0.as_str(), case)
                    }
                    None => false,
                },
                _ => {
                    // 他 component (combinator/pseudo/AttributeOther) は
                    // ruletree build 段で drop 済のはずだが safety net で match fail
                    false
                }
            };
            if !component_matches {
                matches = false;
                break;
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

/// `Component::AttributeInNoNamespace`'s `ParsedCaseSensitivity` (spec-only,
/// "language depends on this" placeholder for the
/// `AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument` case) を、実際に
/// `AttrSelectorOperator::eval_str` へ渡せる `CaseSensitivity` へ解決する。
///
/// upstream `selectors::matching::to_unconditional_case_sensitivity` と同じ
/// 3-way 分岐を model 化しているが、その関数は tree-walk 込みの重い
/// `selectors::Element` trait を要求するため呼べない (raikiri の
/// `StyleElement` は single-element matching 用の縮小 trait — bd
/// raikiri-spike-flln.1 scope、`wall/traits` を跨がない private helper として
/// 再実装)。raikiri は現時点で HTML document のみ対象 (XML/XHTML 未対応) の
/// ため「in html document」は常に true 扱い。「is html element」は
/// [`StyleElement::namespace_uri`] の既存 contract
/// (style_dom.rs: "HTML default namespace returns None (fast path)") を
/// 代理指標として使う — SVG 等 non-HTML namespace の element は
/// case-sensitive 側に倒す。
fn resolve_case_sensitivity<E: StyleElement>(
    parsed: ParsedCaseSensitivity,
    elem: &E,
) -> CaseSensitivity {
    match parsed {
        ParsedCaseSensitivity::CaseSensitive | ParsedCaseSensitivity::ExplicitCaseSensitive => {
            CaseSensitivity::CaseSensitive
        }
        ParsedCaseSensitivity::AsciiCaseInsensitive => CaseSensitivity::AsciiCaseInsensitive,
        ParsedCaseSensitivity::AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument => {
            if elem.namespace_uri().is_none() {
                CaseSensitivity::AsciiCaseInsensitive
            } else {
                CaseSensitivity::CaseSensitive
            }
        }
    }
}

fn specificity_of(selector: &Selector<RaikiriSelectorImpl>) -> Specificity {
    // selectors crate の Selector::specificity は 32-bit packed integer を返す。
    selector.specificity()
}

/// `<img width>` / `<img height>` の HTML presentational-hint 昇格
/// (bd raikiri-spike-5z86.7)。
///
/// # Spec mapping (verbatim, 2026-08-10 直接 fetch した live HTML Standard)
///
/// HTML Living Standard §15.4.3 "Attributes for embedded content and
/// images" (<https://html.spec.whatwg.org/multipage/rendering.html#dimRendering>):
///
/// > The `width` and `height` attributes on an `img` element's dimension
/// > attribute source map to the dimension properties 'width' and 'height'
/// > on the `img` element respectively.
///
/// "maps to the dimension property" (**not** "…(ignoring zero)" — cf. e.g.
/// `<table width>`) は
/// <https://html.spec.whatwg.org/multipage/rendering.html#maps-to-the-dimension-property>
/// が定義: 属性値を [`parse_html_dimension_value`] (HTML LS "rules for
/// parsing dimension values") で parse し、失敗しなければその結果を
/// presentational hint の値として使う。`0` は "ignoring zero" ではないため
/// 有効な hint 値になる。parse 失敗時は hint 自体を作らない (cascade 的には
/// 属性が存在しないのと同じ — 他の source があればそれが勝ち、無ければ
/// property の initial value `auto` のまま)。
///
/// # Non-goals (spec が定義するがこの関数が扱わないこと)
///
/// - **`aspect-ratio` mapping**: 同じ spec 段落が続けて "They similarly map
///   to the aspect-ratio property (using dimension rules) of the `img`
///   element" と述べるが、raikiri-style は `aspect-ratio` property を
///   まだ実装していない ([`crate::property::PropertyValue`] に該当 variant
///   なし) — mapping 先が存在しないので実装しようがない、spec 逸脱ではなく
///   「まだ生えていない property への言及」。
/// - **`dimension attribute source` 間接**:
///   <https://html.spec.whatwg.org/multipage/embedded-content.html#concept-img-dimension-attribute-source>
///   は "initially set to the element itself" で、`<picture>`/`srcset`
///   選択があった場合のみ選ばれた `<source>` 側に切り替わる。raikiri は
///   `<picture>` source 選択を未実装なので、この関数は常に `img` 要素自身の
///   属性を読む — 上記 default と一致する straightforward な subset。
/// - **`embed` / `iframe` / `object` / `video` / `input[type=image]`**: 同じ
///   spec 段落の後続文が他要素にも同じ mapping を適用するが、bd
///   raikiri-spike-5z86.7 の scope narrowing は `img` のみに限定 (最小実装、
///   将来 task の土台という位置づけ)。
///
/// # Cascade origin (retagged `Origin::Author` 2026-08-10 — coordinator
/// spec-lens finding 1、bd raikiri-spike-wo36 が残差を追跡)
///
/// CSS Cascading L5 §6.5 "Precedence of Non-CSS Presentational Hints"
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>) はこの種の hint を
/// **"author presentational hint origin"** という、user origin と author
/// origin の間に位置する独立 origin に置くことを認め、host language が UA-
/// origin か author-origin かを選べるとも書く。HTML LS §15.2 の文言
/// ("author-level zero-specificity presentational hints part of the CSS
/// cascade", <https://html.spec.whatwg.org/multipage/rendering.html#presentational-hints>)
/// は author 寄りだが、raikiri-style の [`Origin`] は `UserAgent` /
/// `Author` の 2 段のみで ([`crate::ruletree`] の module doc 参照)、
/// "author presentational hint origin" 専用の 3 段目は無い。本関数は
/// [`Origin::Author`] を採る — [`cascade_rank`] は `(Author, false) => 1`
/// を `(UserAgent, false) => 0` より**無条件に** (specificity/source_order
/// を問わず) 上位に置くため、真の UA-origin rule (現状
/// `crates/raikiri-html/src/ua/minimal.css` に `img`/`width`/`height` を
/// 宣言する selector は無い) に対しては常に hint が勝つ。
///
/// (旧版はここを [`Origin::UserAgent`] にしていた — 「Author CSS で上書き
/// 可能」という要件は満たしていたが、真の UA-origin rule に対して負ける
/// 方向という逆向きの不整合を持っていた。2-origin model の残差 2 点は
/// この retag で 1 点に減った:)
///
/// 1. spec の完全な順序では hint は「user origin より強い」はず。この
///    crate は user origin を独立に持たず (consumer が渡す
///    `extra_stylesheets` は `Origin::Author` として届く — bd
///    raikiri-spike-m1.22 が定めた 2-origin model、[`crate::ruletree`]
///    module doc 参照)、hint と同じ `Origin::Author` に一律 fold されて
///    いる。retag 後は「consumer 由来の user stylesheet が img の
///    width/height を設定した場合」は同一 origin 内の tie-break
///    (specificity → source_order、下記 2. 参照) に落ちる — 実際の user
///    stylesheet 宣言は具体的な selector を持つ (specificity > 0) のが
///    通常なので実用上は spec 通り user stylesheet が勝つが、理論上
///    zero-specificity な user stylesheet 宣言と衝突すれば 2. と同じ
///    push-order 依存になる。3 段目を追加する over-generalization は
///    本 task の scope 外 (bd raikiri-spike-5z86.7 の「過剰な一般化は
///    避け」)、bd raikiri-spike-wo36 が本来の 3rd origin tier 追加を
///    追跡する。
/// 2. 同一 origin ([`Origin::Author`]) 内での tie-break: 通常は real
///    author 宣言が `beats` の specificity/source_order 勝負で hint に
///    勝つ ([`PRESENTATIONAL_HINT_SPECIFICITY`] = 0 は spec 規定値、
///    real 宣言はほぼ常にそれより高い specificity を持つ)。**例外**:
///    real 宣言が zero-specificity (universal selector 等) かつ、それが
///    その stylesheet の最初の rule (`source_order = 0`,
///    [`crate::ruletree::RuleTree::add_stylesheet`] 参照) の場合、hint
///    の `(rank, specificity, source_order)` と real 宣言のそれが
///    **完全に一致**する ([`PRESENTATIONAL_HINT_SOURCE_ORDER`] doc 参照)。
///    この tie は `beats` の `>=` により「[`pick_winners`] が後から scan
///    した方が勝つ」で決着するため、[`collect_cascaded`] はこの関数を
///    stylesheet rule matching / inline style より**必ず先に** push する
///    — hint が先に scan され、後続の real 宣言が tie を上書きする。
///    テスト
///    `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
///    がまさにこの exact-tie ケースを exercise する。
fn push_img_dimension_hints(elem: &impl StyleElement, decls: &mut Vec<CascadedDecl>) {
    // HTML-namespace gate (Codex §8.3 final-review finding, 2026-08-11):
    // this mapping is HTML LS's own presentational hint, scoped to the HTML
    // namespace — a foreign-namespace element that merely shares the local
    // name "img" (SVG has no `img` element today, but `StyleElement` is a
    // generic trait not tied to any one DOM/parser, so this stays
    // defensive rather than relying on "SVG doesn't currently define one").
    // `namespace_uri()` returns `None` for the HTML default namespace
    // (`style_dom.rs`'s "fast path" doc) — same check/shape as
    // `ruletree.rs`'s `<template>` HTML-only gate
    // (`tag.eq_ignore_ascii_case("template") && elem.namespace_uri().is_none()`,
    // roborev job 292 L2 finding) and the same principle bd
    // raikiri-spike-flln.1 applies to attribute-selector matching this
    // sprint.
    if !elem.tag_name().eq_ignore_ascii_case("img") || elem.namespace_uri().is_some() {
        return;
    }
    if let Some(width) = elem.attr("width").and_then(parse_html_dimension_value) {
        decls.push((
            PropertyValue::Width(LengthOrAuto::Length(width)),
            false,
            Origin::Author,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
        ));
    }
    if let Some(height) = elem.attr("height").and_then(parse_html_dimension_value) {
        decls.push((
            PropertyValue::Height(LengthOrAuto::Length(height)),
            false,
            Origin::Author,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
        ));
    }
}

/// HTML LS "rules for parsing dimension values"
/// (<https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-dimension-values>,
/// verbatim algorithm, 2026-08-10 fetch 確認)。
///
/// 1. 先頭の ASCII whitespace を skip。
/// 2. 直後が ASCII digit でなければ (末尾も含め) 失敗 → `None`。
/// 3. 整数部の連続 digit を集めて 10 進数として解釈。
/// 4. 直後が `.` なら、消費してから続く digit を小数部として集める
///    (`.` の直後が digit でなければ小数部なしとして扱い、位置は `.` の
///    次で確定)。
/// 5. 最終的に「数値の直後の 1 文字」で分類: `%` なら percentage
///    ([`Length::Percent`])、それ以外 (garbage でも文字列終端でも) は
///    length ([`Length::Px`])。
///
/// 数値直後の garbage は失敗にならない — `"42px"` → `Px(42.0)`、
/// `"10.5%rest"` → `Percent(10.5)`。この寛容さは
/// `embedded-content-other.html#dimension-attributes` にある**著者向け**
/// conformance 要件 ("must have values that are valid non-negative
/// integers") とは別物で、UA 側の実際の parse 規則はこちら (dimension
/// value 一般、非負整数だけでなく小数・percentage も受理) — bd
/// raikiri-spike-5z86.7 dispatch prompt の "非負整数" という要約は
/// 説明の簡略化であり、実装はこの spec 本文の algorithm に忠実にした
/// (percentage / 小数を含む)。負値を作る分岐 (`-`/`+` の読み取り) は
/// algorithm 自体に存在しないため、別途の負値拒否は不要。
fn parse_html_dimension_value(input: &str) -> Option<Length> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    // Infra "ASCII whitespace": TAB / LF / FF / CR / SPACE。
    while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b'\n' | b'\x0C' | b'\r') {
        pos += 1;
    }
    if pos >= bytes.len() || !bytes[pos].is_ascii_digit() {
        return None;
    }
    let mut value: f64 = 0.0;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        value = value * 10.0 + f64::from(bytes[pos] - b'0');
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'.' {
        pos += 1;
        if pos < bytes.len() && bytes[pos].is_ascii_digit() {
            let mut divisor: f64 = 1.0;
            loop {
                divisor *= 10.0;
                value += f64::from(bytes[pos] - b'0') / divisor;
                pos += 1;
                if pos >= bytes.len() || !bytes[pos].is_ascii_digit() {
                    break;
                }
            }
        }
    }
    Some(if pos < bytes.len() && bytes[pos] == b'%' {
        Length::Percent(value as f32)
    } else {
        Length::Px(value as f32)
    })
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
///   computed font-size が `ctx.root_font_size`、`lh` の値 (`rlh` の参照値、
///   `used_line_height_length` で絶対長化したもの。`normal` で解決不能なら
///   `None`) が `ctx.root_line_height` (bd raikiri-spike-vxha)。
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
pub(crate) fn resolve_inheritance<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    cascaded: &CascadedArena,
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
    while let Some((id, parent_computed, root_ctx)) = stack.pop() {
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
        if let Some(candidates) = cascaded.candidates(id) {
            apply_winners(candidates, &mut winners, &mut specified);
        }

        // phase 2 + phase 3: 絶対化。root element (element 祖先なし) は `rem` の
        // 基準が phase 2 / phase 3 で異なるため専用 entry point を通す
        // (`SpecifiedValues::finalize_as_root` の doc に spec verbatim)。
        let computed = match &root_ctx {
            Some(ctx) => specified.finalize(&parent_computed, ctx),
            None => {
                // `finalize_as_root` は phase 2 の基準を initial value に固定する
                // (§6.1.1 の "if the element has no parent")。それが正しいのは
                // **`root_ctx == None` ならこの node に element 親が居ない**からで
                // あり、その caller-side invariant を pin しておく:
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

        // 子へ渡す rem/rlh context。root element の phase 2 + 2.5 が終わった
        // 時点で `root_font_size` / `root_line_height` が確定するので、ここで
        // 初めて `Some` になる (bd raikiri-spike-vxha で `root_line_height`
        // を追加)。`used_line_height_length` は
        // `crate::specified::SpecifiedValues::finalize_as_root` が自分の
        // `ctx` を組み立てるのに使う導出と同一 — 両者の一致は
        // `rlh_on_root_element_matches_child_root_line_height_basis` が pin する。
        let child_ctx = match root_ctx {
            Some(ctx) => Some(ctx),
            None if is_element => Some(ResolveContext::with_root_line_height(
                computed.font_size,
                used_line_height_length(computed.line_height, computed.font_size),
            )),
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
        // `child_ids` イテレータを直接 `stack` へ `extend` し、今回追加した
        // 末尾スライスだけを in-place `reverse()` する — 都度捨てる中間
        // `Vec` を経由しない (bd raikiri-spike-75ch)。`child_ctx` は `Copy`
        // (`ResolveContext` の derive) なので closure 内で複数回使い回せる。
        //
        // なぜ document order を保つか: resolve_inheritance 自体の正しさも
        // 訪問順には依存しない — 各 node の computed 値は push 時点で既に
        // 確定している parent_computed / child_ctx だけから決まり、`winners`
        // scratch buffer は各 node の処理前後で完全に drain される (8kn8) の
        // で兄弟の処理順に左右されない。ここで document order を維持して
        // いるのは refactor 前との**挙動の完全一致**のためであり、加えて
        // `winner_does_not_leak_into_next_sibling` 自身の doc comment が
        // 明記する「document order で先行する `<p>` → 後続 `<span>` の向き」
        // という leak 検出方向を、この traversal 順が引き続き満たすため。
        let start = stack.len();
        stack.extend(
            dom.child_ids(id)
                .map(|child_id| (child_id, computed.clone(), child_ctx)),
        );
        stack[start..].reverse();
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
///   parse 出口の展開だけでは post-parse mutation 経路を守れないため
///   (bd raikiri-spike-qzn3 以降この経路は crate 内限定 — 根拠は
///   [`crate::rule::expand_shorthand_into`] doc が canonical)。**shorthand が
///   到達したら既に bug** なので削除可能な dead defensive code ではない。
///
/// 展開後は同一 key の longhand が複数 candidate になるが、[`beats`] の `>=`
/// が「同 rank/spec/order なら後方勝ち」を与えるので §6.1 の order of appearance
/// がそのまま成立する。
///
/// 「展開 arm の書き忘れ」形の壊れ方は **compile-time に強制されている**
/// (bd raikiri-spike-ez7b) — [`crate::rule::expand_shorthand_into`] の match は
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
/// # `candidates` の出所 (bd raikiri-spike-gerj — flat arena 化後)
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
///
/// `pub(crate)` は他 module の doc からの intra-doc link のため — private 化で gate が red (規約 3)。
///
/// `inherited` / 戻り値は `f32` (bd raikiri-spike-e52s で `u16` から格上げ)。
/// table の境界値 (100 / 350 / 550 / 750 / 900) は全て整数だが、`inherited` は
/// fractional weight (`349.5` 等) を保持したまま渡ってくる。丸めずに直接
/// 比較するため行選択は spec §2.2.1 のとおり正確に決まる — 旧 `u16` 実装は
/// parse 段の丸めで `349.5` が `350` に化けてから本関数に渡り、`350 <= w < 550`
/// 行を誤って踏んでいた (詳細: `crate::property::parse_font_weight` doc)。
///
/// # 非有限 `inherited` (`NaN` / `±Inf`) — 本関数は guard しない
///
/// `u16` だった頃は非有限が型で構造的に排除されていたが、`f32` 化 (bd
/// raikiri-spike-e52s) で finiteness は「型で保証」から「呼び出し元の値
/// 検証で保証」に変わった。通常の cascade 経路は
/// `crate::property::parse_font_weight` の `[1, 1000]` range guard により
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
/// **本関数自体には runtime guard を追加しない** (bd raikiri-spike-sxd7、
/// bd raikiri-spike-kfl7 precedent の「非有限 / 範囲外 f32 の guard は sink
/// 境界に置く、resolve 層には置かない」を踏襲)。上記の非対称処理は
/// `resolve_relative_weight_non_finite_inherited_is_asymmetric` test で
/// 現状の挙動として pin 済み。値が実際に `parley::FontWeight::new` へ渡る
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
/// (bolder/lighter と同型、raikiri-spike-4rmu)。
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
///    bd raikiri-spike-zls8、`TextAlign` は bd raikiri-spike-l3wg でそれを
///    済ませた — 前者は payload を destructure して `resolve_font_size` に、
///    後者は [`crate::property::resolve_text_align_match_parent`] に渡している)。
///    **page 経路の、かつ payload 型が
///    `Length` / `LengthOrAuto` / `LineHeight` / `FontWeightValue` /
///    `TextAlign` の 5 つに限れば**、この形の漏れは `page::tests` の
///    `specified_layer_residue` が網羅 match しているので test compile 段で
///    捕まる (bd raikiri-spike-awjx)。それ以外 (`BorderStyle` / `BorderColor`
///    / `DisplayValue` / `PositionValue` / `BoxSizing` / `ContentComponent`)
///    は同検出器も `_` で捨てており、`Border` struct の field 追加も
///    field access で読んでいるため捕まらない。compile error になるのも
///    test target であって本関数ではない。
/// 2. **本関数を呼ばない新しい entry point** — ygl0 の regression はこの形
///    だった (`cascade_page` が `apply_value` を通らなかった)。CSS Paged Media 3
///    §6 の margin-box cascade は page context を継承元とする第 3 の経路になる。
///    exhaustive match は「経路の数え上げ」を強制しない。
///
///    bd raikiri-spike-7m33 でこの穴を **型で狭めた** (完全には塞いでいない)
///    — 本関数の戻り値は生の [`PropertyValue`] ではなく
///    [`ResolvedAgainstInherited`]。その型の doc「narrowed, not closed」節が
///    canonical な記述 (何を防ぎ、何を防がないか、残余が
///    bd raikiri-spike-m4.2 に切り出されていること) を持つので、ここでは
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
/// font-size を基準に絶対化 + style gating を行う (bd raikiri-spike-sshp)。
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
/// # `TextAlign::MatchParent` は本関数が解決する (raikiri-spike-l3wg)
///
/// [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent) は `inherited`
/// だけで解ける — CSS Text 3 §6.1 `#valdef-text-align-match-parent` の
/// 「実の親を持つ」半分 (root element の "computes to start" は対象外、下記注記)
/// — ので本関数の `TextAlign` arm が
/// [`crate::property::resolve_text_align_match_parent`] へ `inherited.text_align` +
/// `inherited.direction` を渡して解決する。以前 (raikiri-spike-l3wg 着手前) は raikiri
/// が `direction` を computed 層に持たなかったため未実装だった (origin:
/// raikiri-spike-ygl0 §8.2 spec lens F1)。
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
/// raikiri-spike-l3wg で解消され、`page::tests` の
/// `page_declarations_carry_no_specified_layer_residue` (旧
/// `page_declarations_carry_exactly_one_specified_layer_residue`) が
/// pin する。
/// なお `Percent` は「未解決」ではない — box property の computed value は
/// percentage のままである (CSS Paged Media 3 §6 の "Percentage values on the
/// margin and padding properties are relative to the dimensions of the
/// containing block" = used 層の入力。引用は canonical 側)。element 経路の
/// [`crate::resolve::resolve_length_percentage`] と同じ扱い。
///
/// # 戻り値が生の [`PropertyValue`] ではなく [`ResolvedAgainstInherited`] な理由
///
/// bd raikiri-spike-7m33 — 上記「この guard が守らない範囲」§2
/// (本関数を呼ばない新しい entry point) を型で狭めるため。詳細は
/// [`ResolvedAgainstInherited`] の doc を参照。
///
/// # `ctx` の caller contract (bd raikiri-spike-yh3w)
///
/// `ctx.root_line_height` は `inherited` から導出したもの
/// (`used_line_height_length(inherited.line_height, inherited.font_size)`)
/// を渡すこと — `FontSize` arm の `lh`/`rlh` 解決 (下記 arm 参照) がこの
/// 一致を前提にしている。唯一の呼び手 [`crate::page::cascade_page`] はこれを
/// 一度だけ構築し、本関数と phase 3 ([`crate::page`] の `absolutize_in_page_context`)
/// の両方に使い回す (`inherited` は関数全体で不変なので、二重に計算しても
/// 同じ値になる — 呼び手の doc 参照)。
pub(crate) fn resolve_against_inherited(
    value: PropertyValue,
    inherited: &ComputedValues,
    ctx: &ResolveContext,
) -> ResolvedAgainstInherited {
    ResolvedAgainstInherited(match value {
        // CSS Fonts 4 §2.2.1 "Relative Weights"
        // <https://www.w3.org/TR/css-fonts-4/#relative-weights>: `bolder` /
        // `lighter` は継承元の computed weight に対して解決される。ここで
        // `Absolute` に落とすので戻り値に relative keyword は残らない
        // (`Absolute(f32)` → `f32` → `Absolute(f32)` の round-trip は無損失)。
        PropertyValue::FontWeight(fw) => PropertyValue::FontWeight(FontWeightValue::Absolute(
            resolve_relative_weight(fw, inherited.font_weight),
        )),
        // `font-size` は継承元の computed font-size だけで解ける (bd
        // raikiri-spike-zls8)。`crate::resolve::resolve_font_size` に funnel し、
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
        // - `lh` / `rlh` (bd raikiri-spike-yh3w): §6 は `lh`/`rlh` を規定して
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
        // CSS Text 3 §6.1 `#valdef-text-align-match-parent` (raikiri-spike-l3wg)。
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
        // CSS Fonts 4 §2.5 `<relative-size>` (`larger` / `smaller`、
        // raikiri-spike-4rmu): `bolder` / `lighter` と同型、継承元の computed
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
        // `FontFamily` と同型 (raikiri-spike-l3wg)。
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
        | PropertyValue::Direction(_)
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
    })
}

/// [`resolve_against_inherited`] (phase 2) を通過済であることを **型で**示す
/// wrapper。tuple field は本 module (`cascade`) に private — 他 module は
/// [`resolve_against_inherited`] を呼ぶ以外にこの型の値を作れない
/// (bd raikiri-spike-7m33)。
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
/// — 残る「経路の数え上げ」不能性は bd raikiri-spike-m4.2 (margin-box cascade
/// 実装 task の acceptance criteria) に切り出した。[`resolve_against_inherited`]
/// の doc「この guard が守らない範囲」§2 も参照。
///
/// # test 用の裏口 (`Self::for_test`)
///
/// `page::tests` には phase 3 を意図的に phase 2 抜きで直接駆動する既存 test
/// 群がある (`phase_3_variant_classification_matches_the_documented_counts` /
/// `absolutize_in_page_context_shorthand_fall_throughs` /
/// `absolutize_in_page_context_font_size_relative_safety_net` —
/// いずれも「structurally unreachable だが `pub(crate)` 関数は直接駆動できる」
/// という既存の defense-in-depth 方針、bd raikiri-spike-ez7b / raikiri-spike-4rmu
/// 系列の precedent)。これらが本型導入後も raw payload を直接検査できるよう、
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

    /// Phase 2 を通過済の値を覗き見る (所有権を取らない版)。`crate::page` の
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
/// 行うことは decision raikiri-spike-082k により禁じられている (`padding: 2em` の
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
        // CSS Backgrounds 3 §2.2 (raikiri-spike-0vv.7)。sibling `Color` と対称的な
        // 単純代入 (non-inherited、per-node で cascade winner を直接反映)。
        PropertyValue::BackgroundColor(c) => target.background_color = c,
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        // CSS Fonts 4 §2.5 `<relative-size>` (`larger` / `smaller`、
        // raikiri-spike-4rmu)。`font-weight` の `bolder` / `lighter` arm
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
        //   (decision raikiri-spike-082k、本関数冒頭の doc 参照) が、
        //   `larger` / `smaller` は基準が「親の computed font-size」のみで
        //   自 node の他 winner に依存しないため、`font-weight` と同じく
        //   ここ (phase 1) で解決してよい。解決結果は `Length::Px` — 通常の
        //   author 指定 px 値と区別が付かなくなり、phase 2 (`resolve_font_size`
        //   の `Px` arm は identity) を通しても二重適用にならない。
        // - 全 `Length` variant を OR-pattern で受ける下の抽出は
        //   「実際には常に `Px`」を panic-free に表現したもの — reviewer-security
        //   の panic surface 排除方針 (`Margin` shorthand fall-through arm と同じ
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
            // Re-examined for bd raikiri-spike-yh3w (which made `font-size:
            // 1lh` / `1rlh` parse-accepted, removing the *previous* reason
            // this was unreachable — that `parse_font_size` dropped them at
            // parse time). The `Lh`/`Rlh` arms remain unreachable, but for a
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
        // CSS Fonts 4 §2.2 (raikiri-spike-5iy + raikiri-spike-17s8)。specified
        // value は `FontWeightValue` (relative keyword を保持)、computed value
        // は resolve 済み `f32` (raikiri-spike-e52s で `u16` から格上げ) —
        // `bolder` / `lighter` はここで絶対値に落とす。
        //
        // 継承値の出所: `target` は直前に `SpecifiedValues::inherit_from(parent)`
        // で seed されており (`resolve_inheritance` 参照)、`font_weight` は
        // inherited property なので **この時点の `target.font_weight` は親の
        // computed font-weight そのもの**。`SpecifiedValues` が
        // `font_weight: f32` を「既に computed-equivalent」として持つのはこの
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
        // なお本 arm は `apply_value` 中の read-modify-write の 1 つ (raikiri-spike-4rmu
        // で `FontSizeRelative` arm が 2 つ目に加わった。それ以外はすべて冪等な
        // 単純代入)。`Padding` / `Margin` / `Border` shorthand fall-through arm
        // のような二重適用経路を font-weight に足すと `bolder` が 400 → 700 → 900
        // と複合するため、上記 2 invariant を崩す変更は不可。`FontSizeRelative` も
        // 同じ理由で二重適用経路を持たない (`FontSize` と同一 `PropertyKey`
        // を共有し slot は 1 つ、詳細は該当 arm の comment)。
        //
        // **契約 (raikiri-spike-ygl0)**: 継承元依存の解決を持つ property を新しく
        // 追加するときは、本 arm だけでなく sibling の
        // `resolve_against_inherited` にも arm を足すこと — そちらは
        // `PropertyValue` を返す形で同じ解決を提供し、
        // `crate::page::cascade_page` (`apply_value` を通らない第 2 の public
        // entry point) が使う。両者とも wildcard 無しの exhaustive match なので
        // variant 追加時は compiler が 2 経路を数え上げさせる。
        //
        // **例外 (raikiri-spike-l3wg)**: `text-align: match-parent` は
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
        // string-set は M5 static-side β (raikiri-spike-m5.3、CSS GCPM 3 §1.1.1)。
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
        //
        // **`match-parent` はここでは解決しない** (raikiri-spike-l3wg) — この
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
        // direction は CSS Writing Modes 4 §2.1 (raikiri-spike-l3wg)。
        // inherited property、computed value = specified value (相対解決なし) —
        // text-align と同じく単純代入で十分。
        PropertyValue::Direction(d) => target.direction = d,
        // CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical>
        // padding physical longhand (raikiri-spike-0vv.6)。
        // 4 side を独立に上書き。shorthand `PropertyValue::Padding` は
        // `crate::rule::expand_shorthand_into` により parse 出口と element cascade 入口
        // (`collect_cascaded`) の両方で 4 longhand に展開されるため、cascade 段に
        // 届く declaration は per-side longhand
        // のみ = shorthand/longhand の cross-key dependency が消え、winner の
        // 適用順に依存しない per-key determinism が成立する
        // (raikiri-spike-5nc、margin 0vv.5 の parse-time expansion model に migrate)。
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
        // 4 longhand margin sides (raikiri-spike-0vv.5、CSS Box 3 §3.1)。
        // shorthand `PropertyValue::Margin` は `crate::rule::expand_shorthand_into`
        // により parse 出口と element cascade 入口の両方で 4 longhand に展開されるため、
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
        // `crate::rule::expand_shorthand_into` の call site 1 / 2 (parse 出口と
        // element cascade 入口) が担保しており、その担保のうち「展開 arm の
        // 書き忘れ」は同関数の exhaustive match で compile-time に排除されている
        // (bd raikiri-spike-ez7b。残る範囲は同関数 doc の
        // 「この guard が守らない範囲」節)。振る舞い自体は
        // `apply_value_direct_margin_shorthand_fall_through` test が直接叩いて pin。
        PropertyValue::Margin(sides) => target.margin = sides,
        // CSS Backgrounds 3 §3.3/§3.2/§3.1 border physical longhand
        // (raikiri-spike-0vv.12)。4 side × 3 sub-property の 12 arm。shorthand
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
        // その unreachability の compile-time 強制 (bd raikiri-spike-ez7b) は
        // `Margin` arm の comment 参照。
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
    use crate::computed::INITIAL_FONT_SIZE_PX;
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

    // ── class / id / attribute selector matching (bd raikiri-spike-flln.1) ──
    //
    // Spec: CSS Selectors Level 4 — class selector
    // <https://www.w3.org/TR/selectors-4/#class-html>, ID selector
    // <https://www.w3.org/TR/selectors-4/#id-selectors>, attribute selector
    // <https://www.w3.org/TR/selectors-4/#attribute-selectors>.
    //
    // `cascade_doc` (above) has no way to set `class`/`id`/arbitrary attrs —
    // it only threads `inline_style` through `push_element` — so these tests
    // build the `TestDoc` directly via `push_element` + `TestDoc::set_attr`.

    #[test]
    fn class_selector_applies_declaration() {
        // Acceptance (bd raikiri-spike-flln.1): `.chapter-title { font-weight:
        // bold }` applied to `<p class="chapter-title">` — `font-weight: bold`
        // computes to 700.0 (property.rs `parse_font_weight`).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter-title { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "chapter-title");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
    }

    #[test]
    fn class_selector_does_not_match_element_without_the_class() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter-title { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "intro"); // different token

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_weight, 400.0,
            "initial, rule must not apply"
        );
    }

    #[test]
    fn class_selector_matches_one_token_among_several() {
        // `class="a b c"` — HTML-spec ASCII whitespace split
        // (`StyleElement::has_class` doc), `.b` must match the middle token.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".b { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "a b c");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
    }

    #[test]
    fn class_selector_is_case_sensitive() {
        // CSS Selectors L4 class-html: HTML class matching in standards mode
        // is case-sensitive (`StyleElement::has_class` default impl does an
        // exact token compare, no ASCII-case-folding).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".Foo { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo"); // different case

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_weight, 400.0,
            "initial, case must not fold"
        );
    }

    #[test]
    fn id_selector_applies_declaration() {
        // Acceptance (bd raikiri-spike-flln.1): `#header { ... }` applied to
        // `<div id="header">`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#header { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "id", "header");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn id_selector_does_not_match_different_id() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#header { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "id", "footer");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_exists_selector_applies_declaration() {
        // Acceptance (bd raikiri-spike-flln.1): `[data-foo]` matches any
        // element carrying that attribute, regardless of its value.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "anything");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_exists_selector_does_not_match_when_attr_absent() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo] { color: red }");
        let div = doc.push_element(0, "div", None); // no data-foo at all

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_exists_selector_does_not_match_empty_value_attr() {
        // `StyleElement::attr` 契約 ("Empty string is normalised to `None`",
        // style_dom.rs doc) の帰結を明示的に pin する — 実 DOM 上は
        // `data-foo=""` も「属性は存在する」が、raikiri の `attr()` 契約は
        // 空文字を無指定と同一視するため `[data-foo]` はここでは match しない。
        // これは本 task が導入した挙動ではなく、既存の `StyleElement` 契約を
        // そのまま matcher に伝播させた結果 (`elem.attr(...).is_some()`)。
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_exists_selector_mixed_case_matches_html_element_via_lowercased_key() {
        // HTML LS "case-sensitivity of selectors"
        // (https://html.spec.whatwg.org/multipage/semantics-other.html#case-sensitivity-of-selectors):
        // attribute names on HTML elements in HTML documents are
        // ASCII-lowercased — html5ever already lower-cases them at parse
        // time (`raikiri-html::sink::wire_side_tables` stores whatever case
        // html5ever produced, unmodified). So a selector written with mixed
        // case, `[Data-Foo]`, must still match an HTML (default-namespace)
        // element whose stored attribute name is already-lowercased
        // `data-foo`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[Data-Foo] { color: red }");
        let div = doc.push_element(0, "div", None); // default namespace = HTML
        doc.set_attr(div, "data-foo", "anything");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_exists_selector_mixed_case_uses_original_case_for_foreign_namespace_element() {
        // Foreign-namespace (SVG/MathML) elements are NOT covered by HTML
        // LS's "attributes on HTML elements in HTML documents" lowercasing
        // scope — html5ever's "adjust foreign attributes" step can restore
        // specific attributes to their original mixed case (e.g. `viewBox`),
        // and `wire_side_tables` stores whatever case html5ever produced,
        // unmodified. A selector written `[Data-Foo]` against such an
        // element must use the *original-case* lookup key, not the
        // lowercased one.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[Data-Foo] { color: red }");
        let svg_el = doc.push_element(0, "rect", None);
        doc.set_namespace(svg_el, "http://www.w3.org/2000/svg");
        doc.set_attr(svg_el, "Data-Foo", "anything"); // original mixed case

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[svg_el].color, RED);
    }

    #[test]
    fn attribute_exists_selector_mixed_case_does_not_fall_back_to_lowercase_for_foreign_namespace_element()
     {
        // Same shape as the sibling test above, but the foreign-namespace
        // element carries only the *lowercased* attribute name — proving
        // the lookup is genuinely gated on the original-case key for
        // foreign elements, not silently trying both keys.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[Data-Foo] { color: red }");
        let svg_el = doc.push_element(0, "rect", None);
        doc.set_namespace(svg_el, "http://www.w3.org/2000/svg");
        doc.set_attr(svg_el, "data-foo", "anything"); // lowercased — wrong key for this element

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[svg_el].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_value_exact_match_selector_applies_declaration() {
        // Acceptance (bd raikiri-spike-flln.1): `[data-foo="bar"]` exact-match
        // variant.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "bar");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_value_exact_match_selector_does_not_match_different_value() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "baz");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_value_exact_match_is_case_sensitive_for_data_attr() {
        // CSS Selectors L4 attribute-selectors: default case-sensitivity
        // (no `i`/`s` flag) depends on the document language; `data-*` is not
        // in HTML's ASCII-case-insensitive attribute list, so it resolves to
        // `ParsedCaseSensitivity::CaseSensitive` at parse time (selectors
        // crate `AttributeFlags::to_case_sensitivity`) — no
        // `resolve_case_sensitivity` branching is even reached for this case.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "BAR"); // different case

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_value_case_insensitive_flag_i_matches_regardless_of_case() {
        // `[foo="bar" i]` — explicit `i` flag forces ASCII-case-insensitive
        // matching regardless of the attribute's document-language default
        // (CSS Selectors L4 attribute-selectors, `AttributeFlags::AsciiCaseInsensitive`).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\" i] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "BAR");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_selector_style_local_name_matches_element_with_inline_style() {
        // `StyleElement::attr`'s doc contract requires overrides to keep
        // handling `local == "style"` by delegating to
        // `inline_style_source()`; `TestElementRef::attr` (bd
        // raikiri-spike-flln.1) does this, so `[style]` — an ordinary
        // existence attribute selector whose local name happens to be
        // `style` — must match any element carrying an inline `style="…"`.
        // `font-weight` (not touched by the inline `color: blue`) is the
        // observable, since inline style otherwise always outranks any
        // stylesheet rule (`INLINE_SPECIFICITY`) regardless of whether
        // `[style]` itself matched.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[style] { font-weight: bold }");
        let p = doc.push_element(0, "p", Some("color: blue"));

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
    }

    #[test]
    fn attribute_selector_style_does_not_match_element_without_inline_style() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[style] { font-weight: bold }");
        let p = doc.push_element(0, "p", None); // no inline style

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 400.0);
    }

    #[test]
    fn compound_type_and_class_selector_requires_both() {
        // `p.chapter-title` — compound selector, AND semantics: both the type
        // and class component must match the same element.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p.chapter-title { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "chapter-title");
        let div = doc.push_element(0, "div", None); // wrong tag, same class
        doc.set_attr(div, "class", "chapter-title");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_weight, 700.0,
            "p.chapter-title must match <p class=chapter-title>"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[div].font_weight, 400.0,
            "div.chapter-title selector must not match <div class=chapter-title> (wrong tag)"
        );
    }

    #[test]
    fn id_selector_specificity_beats_class_selector() {
        // CSS Cascading L4 §6.1 sort criterion (b): higher specificity wins.
        // ID (0,1,0,0) > class (0,0,1,0) — both target the same element via
        // separate rules, later source order for the loser to make sure the
        // win is attributable to specificity, not source order.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".foo { color: blue } #bar { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "foo");
        doc.set_attr(div, "id", "bar");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[div].color, RED,
            "id selector must win over class selector"
        );
    }

    #[test]
    fn match_simple_selectors_rejects_unsupported_component_via_safety_net() {
        // combinator/pseudo-class components never reach `match_simple_selectors`
        // in the real pipeline — `ruletree.rs`'s `is_supported_selector_list`
        // drops any rule containing one at `add_stylesheet` time (pinned by
        // `ruletree::tests::pseudo_class_selector_still_dropped`). This test
        // calls `match_simple_selectors` directly — both it and `parse_selector_list`
        // are reachable from this `#[cfg(test)] mod tests` (`use super::*` /
        // `crate::parse_selector_list`) — to exercise the `_ => false`
        // safety-net arm defensively, per its own doc comment.
        let list = crate::parse_selector_list("p:hover").expect("selector parses");
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", None);
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            match_simple_selectors(&list, &elem),
            None,
            "NonTSPseudoClass component must fall through the safety net"
        );
    }

    #[test]
    fn resolve_case_sensitivity_html_default_namespace_folds_case_for_html_case_insensitive_attr() {
        // `type` is on HTML's ASCII-case-insensitive attribute list (the
        // selectors crate's generated `ascii_case_insensitive_html_attributes`
        // set) — with no explicit `i`/`s` flag, `[type=...]` parses to
        // `ParsedCaseSensitivity::AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument`,
        // which `resolve_case_sensitivity` must fold to ASCII-case-insensitive
        // for an element in the default (HTML) namespace — `TestElementRef`
        // returns `None` from `namespace_uri()` unless overridden via
        // `TestDoc::set_namespace`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[type=\"text\"] { color: red }");
        let input = doc.push_element(0, "input", None);
        doc.set_attr(input, "type", "TEXT"); // different case than the selector

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[input].color, RED,
            "[type=...] must ASCII-case-fold under the HTML default"
        );
    }

    #[test]
    fn resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive() {
        // Same `[type=...]` shape as the sibling test above, but the element
        // carries an explicit non-HTML namespace (SVG) — `resolve_case_sensitivity`
        // must fall back to case-sensitive matching for it (own doc comment:
        // "SVG 等 non-HTML namespace の element は case-sensitive 側に倒す").
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[type=\"text\"] { color: red }");
        let input = doc.push_element(0, "input", None);
        doc.set_attr(input, "type", "TEXT");
        doc.set_namespace(input, "http://www.w3.org/2000/svg");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[input].color,
            ComputedValues::initial().color,
            "non-HTML-namespace element must not case-fold [type=...]"
        );
    }

    /// `INLINE_SPECIFICITY` (cascade.rs doc, CSS Cascading L4 §6.1
    /// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>: "declarations that
    /// do not belong to a style rule ... are considered to have a specificity
    /// higher than any selector") pin — bd raikiri-spike-nvhy.
    ///
    /// # なぜ hardcoded 算術 assert ではなく実 parse なのか
    ///
    /// upstream `selectors` crate (v0.39) は `id_selectors << 20 |
    /// class_like_selectors << 10 | element_selectors` の 32-bit packed
    /// specificity を持ち、各 field は `cmp::min(field, MAX_10BIT)` で
    /// **10-bit に飽和** する
    /// (`selectors-0.39.0/builder.rs` の `MAX_10BIT` / `impl From<Specificity>
    /// for u32` — 挙動理解のための参照であり、raikiri-style 側の実装判断は
    /// この private 定数からではなく selectors の**公開 API** から導いている)。
    /// `MAX_10BIT` はモジュール private (`pub(crate)` にすらなっていない) で
    /// 外部 crate から import できないため、`MAX_PACKED_SPECIFICITY` を
    /// upstream の型から直接 const-derive することは**そもそも不可能**
    /// (許可/禁止の問題ではなく、単に import できる定数が存在しない)。
    ///
    /// 代わりに `Selector::specificity()` を使って現実に到達可能な最大値を
    /// 実測する: 現行の 10-bit 飽和点 (1023) を大きく超える数の ID / class
    /// selector を含む compound selector を構築し、飽和後の実値を読み戻す。
    /// こうすれば upstream が将来 field 幅を広げても (例: issue 本文が挙げる
    /// 11-bit 化)、本 test は**その時点の upstream 実装が実際に返す値**を
    /// 再測定し続けるので、`INLINE_SPECIFICITY` を超えた瞬間に fail する —
    /// 「今の幅を前提にした算術の pin」より頑丈 (bd raikiri-spike-nvhy 提案の
    /// うち、hardcoded const assert ではなく実測 test を採る方の案)。
    ///
    /// # 未 cover: cascade 経由の end-to-end pin (M1.4 執筆時点では実装不可だった)
    ///
    /// bd issue が挙げるもう 1 案 (`<p id class>` に対する高 specificity
    /// selector と inline style を実際に cascade させ、inline が勝つことを
    /// 見る e2e test) は M1.4 執筆時点では構築できなかった: 当時の
    /// `ruletree.rs` `is_type_or_universal_only` が id/class を含む selector
    /// を rule tree 構築時点で drop し、当時の `match_by_tag` も
    /// type/universal 以外の component を持つ selector を一致させなかった
    /// (M1.4 は type + universal selector のみ対応)。したがって本 test は
    /// 「numeric な不変条件そのもの」を `crate::parse_selector_list` 経由で // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-o9h6)
    /// 直接 pin するに留めた。
    ///
    /// bd raikiri-spike-flln.1 で class/id/attribute selector matching が
    /// 実装され (`is_type_or_universal_only` → `is_supported_selector_list`
    /// rename、`match_by_tag` → `match_simple_selectors` rename +
    /// `StyleElement` 対応)、上記の e2e test を阻んでいたブロッカーは解消
    /// 済み。e2e test 自体の追加は本 test の scope 外のまま —
    /// bd raikiri-spike-7ejc に残す。
    ///
    /// selector 内の class / pseudo-class 数も併せて増やし
    /// (class_like_selectors field)、id field 単独ではなく複数 field が
    /// 同時に飽和する構成にしている。element_selectors field (type selector /
    /// pseudo-element 由来) は 1 compound selector につき type selector を
    /// 1 つしか持てないが、descendant combinator で compound を連結すれば
    /// compound ごとに `Component::LocalName` が積み上がるため、この field も
    /// 公開 API 経由で飽和させられる (`RaikiriSelectorParser` は
    /// pseudo-element 未サポート — `crate::PseudoElem` は uninhabited — // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-o9h6)
    /// だが combinator 連結には無関係)。3 field 全てを飽和させると理論上の
    /// packed 最大値 `0x3FFF_FFFF` (margin 1、実測値) に一致する — これは
    /// 本 const 直上の doc の「margin はちょうど 1」と整合する。
    #[test]
    fn inline_specificity_exceeds_max_reachable_packed_specificity() {
        // 現行 10-bit 飽和点 (1023) を十分に超える数。id field は 4096 個、
        // element field は type-chain 1201 個 (下記) で現行 field を確実に
        // 飽和させる。この余裕はあくまで「現行 10-bit field を確実に飽和
        // させる」ためのものであり、upstream が将来 field 幅を広げた場合に
        // **その新しい幅でも飽和し続ける**ことまでは保証しない (例えば
        // 12-bit = 4095 まで広がれば、この個数では飽和しきらない)。
        // それでも `measured_specificity` は selectors crate の公開 API から
        // 都度実測する値なので、幅が変わって挙動が変化したこと自体は
        // 検知できる — 「理論上の最大値と一致し続ける」のではなく
        // 「upstream の実装変化を都度観測する」ことが本 test の pin 機構。
        const FIELD_REPEAT: usize = 4096;
        // element_selectors field は 1 compound selector につき type
        // selector を 1 つしか持てないが、descendant combinator で compound
        // を連結すれば compound ごとに LocalName 分が積み上がる。
        // `"div "` (末尾 space = descendant combinator) を 1200 回連結した
        // 直後に最終 compound `div#a#a...#a.b.b...:hover:active` を置き、
        // id / class / element の 3 field を同時に飽和させる。
        const TYPE_CHAIN_REPEAT: usize = 1200;
        let type_chain: String = "div ".repeat(TYPE_CHAIN_REPEAT);
        let ids: String = "#a".repeat(FIELD_REPEAT);
        let classes: String = ".b".repeat(FIELD_REPEAT);
        let selector_str = format!("{type_chain}div{ids}{classes}:hover:active");
        let list = crate::parse_selector_list(&selector_str).expect("maximal selector must parse");
        let measured_specificity: Specificity = list
            .slice()
            .iter()
            .map(|s| s.specificity())
            .max()
            .expect("selector list is non-empty");

        assert!(
            INLINE_SPECIFICITY > measured_specificity,
            "INLINE_SPECIFICITY ({INLINE_SPECIFICITY:#x}) は selectors crate の \
             公開 API (Selector::specificity) で実測した到達可能最大値 \
             ({measured_specificity:#x}) を上回らなければならない — CSS Cascading L4 \
             §6.1 の 'higher than any selector' 要件。selectors crate の \
             packed-specificity field 幅が変わった signal (bd raikiri-spike-nvhy \
             参照、cascade.rs INLINE_SPECIFICITY doc)。"
        );
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
        // 同一 rule 内で同じ property が 2 回 — CSS Cascading L4 §6.1 "Order of
        // Appearance" <https://www.w3.org/TR/css-cascade-4/#cascade-sort>:
        // "The last declaration in document order wins."
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
    /// declaration を 2 回**適用する。`bolder` は `apply_value` 中で唯一の
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
    ///   `pick_winners` 冒頭の debug_assert が先に落ちる。`bolder` の二重適用
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
            r.computed[span].font_weight, 700.0,
            "font-weight: bolder が 2 回適用された (slot leak による二重 drain)"
        );
    }

    /// bd raikiri-spike-gerj: `collect_cascaded` の出力が per-node
    /// `HashMap<StyleNodeId, Vec<CascadedDecl>>` から flat arena +
    /// `HashMap<StyleNodeId, Range<usize>>` (`CascadedArena`) に変わったあと、
    /// per-node grouping / 内部順序 / 「候補 0 件なら entry 無し」の 3 つが
    /// 旧実装と不変であることを直接 pin する。
    ///
    /// 3 兄弟 `<p>` を作り、うち 2 つ (`p1`/`p3`) には inline style も足す —
    /// 「stylesheet 2 rule → inline 1 件」の混在順序 (旧実装のまま:
    /// stylesheet 由来が先、inline が最後) を見るため。`<hr>` は
    /// マッチする rule も inline style も持たない — 旧実装の
    /// `if !per_node.is_empty() { out.insert(..) }` と同じ「候補が無ければ
    /// map に entry を作らない」契約を CascadedArena も引き継いでいることを
    /// 確認する。
    ///
    /// ranges の非重複性は flat arena 特有の新しい不変条件 — 個別 `Vec` には
    /// 存在しなかった「他 node の区間と重なってはいけない」という要求で、
    /// `CascadedArena::candidates` が正しい slice を返す前提そのもの
    /// (raikiri-spike-8kn8 が警告する「global index space を渡すと壊れる」
    /// ハザードの、arena 版の再発防止)。2 つの assertion で役割が分かれる:
    /// `windows(2)` が三者間の pairwise overlap を検査し、末尾の
    /// `assert_eq!` (arena 全長 == 3 区間長の和) が「どの named range にも
    /// 属さない迷子 slot」の有無を検査する — 前者だけでは後者は捕まらない。
    #[test]
    fn collect_cascaded_groups_are_unchanged_by_flat_arena_refactor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red } p { background-color: blue }");

        let p1 = doc.push_element(0, "p", Some("display: block"));
        let p2 = doc.push_element(0, "p", None);
        let empty = doc.push_element(0, "hr", None);
        let p3 = doc.push_element(0, "p", Some("display: inline"));

        let tree = build_rule_tree(&doc);
        let mut arena = CascadedArena::new();
        collect_cascaded(&doc, doc.root_id(), &tree, &mut arena);

        let id = |i: usize| StyleNodeId::new(i as u64);

        // `hr` matches no rule and has no inline style — old code's
        // `if !per_node.is_empty()` guard meant no map entry at all; the
        // arena must not create a zero-length range for it either.
        assert!(
            arena.candidates(id(empty)).is_none(),
            "element with zero candidate declarations must get no arena entry"
        );

        // p2: 2 stylesheet decls, no inline — order = rule/source order.
        let p2c = arena.candidates(id(p2)).expect("p2 has 2 stylesheet decls");
        assert_eq!(p2c.len(), 2);
        assert_eq!(p2c[0].0, PropertyValue::Color(RED));
        assert_eq!(p2c[1].0, PropertyValue::BackgroundColor(BLUE));
        assert_eq!(p2c[0].4, 0, "first rule keeps its source_order");
        assert_eq!(p2c[1].4, 1, "second rule keeps its source_order");

        // p1 / p3: same 2 stylesheet decls, PLUS inline style appended last
        // (collect_cascaded pushes stylesheet rules before inline style).
        let p1c = arena.candidates(id(p1)).expect("p1 has decls");
        assert_eq!(p1c.len(), 3, "2 stylesheet decls + 1 inline, inline last");
        assert_eq!(p1c[0].0, PropertyValue::Color(RED));
        assert_eq!(p1c[1].0, PropertyValue::BackgroundColor(BLUE));
        assert_eq!(p1c[2].0, PropertyValue::Display(DisplayValue::Block));
        assert_eq!(p1c[2].3, INLINE_SPECIFICITY);
        assert_eq!(p1c[2].4, INLINE_SOURCE_ORDER);

        let p3c = arena.candidates(id(p3)).expect("p3 has decls");
        assert_eq!(p3c.len(), 3);
        assert_eq!(p3c[2].0, PropertyValue::Display(DisplayValue::Inline));

        // Stylesheet-only decls (p1/p2/p3 all matched the same 2 `p` rules)
        // carry identical specificity to each other — cross-node consistency
        // the old shared-selector-per-rule code guaranteed too.
        assert_eq!(p1c[0].3, p2c[0].3);
        assert_eq!(p2c[0].3, p3c[0].3);

        // Ranges must not overlap — a flat arena has to hold this invariant
        // that per-node `Vec`s never needed to: if two nodes' ranges ever
        // overlapped, `candidates(id)` would silently hand `pick_winners` a
        // slice containing another node's declarations too
        // (raikiri-spike-8kn8's "global index space" hazard, arena-shaped).
        //
        // The `windows(2)` loop below only checks pairwise overlap among the
        // three explicitly-named nodes (p1/p2/p3) — it would miss a stray
        // arena slot that belongs to no range, or one double-counted across
        // two ranges. The load-bearing check for "no slot unaccounted for"
        // is the trailing `assert_eq!` after the loop: it compares the
        // arena's total length against the sum of every named range's
        // length, so any leaked/duplicated/orphaned slot shows up as a
        // length mismatch even if no two of the three named ranges overlap
        // each other directly.
        let mut ranges: Vec<_> = [p1, p2, p3]
            .iter()
            .map(|&n| arena.ranges.get(&id(n)).unwrap().clone())
            .collect();
        ranges.sort_by_key(|r| r.start);
        for w in ranges.windows(2) {
            assert!(
                w[0].end <= w[1].start,
                "per-node ranges must not overlap: {:?} vs {:?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(
            arena.decls.len(),
            ranges.iter().map(|r| r.len()).sum::<usize>(),
            "every arena slot belongs to exactly one node's range"
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

    // ── `lh` / `rlh` (CSS Values 4 §6.1.1, bd raikiri-spike-vxha) ──────────

    /// `padding: 1lh` は **自 node の** used line-height (own font-size ×
    /// `<number>`) を基準にする — CSS Values 4 §6.1.1 `lh`。
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

    /// `line-height: normal` (initial value) の下で `1lh` を使うのは common
    /// case — real font metrics が無いので padding の spec initial `0` に
    /// 倒す (`crate::resolve::resolve_length_percentage` doc、cleanroom: // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-o9h6)
    /// 比率を捏造しない)。
    #[test]
    fn lh_falls_back_to_zero_when_own_line_height_is_normal() {
        let cv = cascade_doc("", "div", Some("padding: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    /// roborev-refine iter 1 Finding A regression pin: `margin-top: 1lh`
    /// under the extremely common `line-height: normal` configuration must
    /// compute to `Px(0.0)` — margin's true spec initial (CSS Box 3 §3.1) —
    /// **not** `Auto`. `Auto` would silently trigger real taffy auto-margin
    /// layout (space distribution / centering) with no spec basis, which is
    /// the concrete failure mode the fix (`resolve_margin_length_or_auto`,
    /// bd raikiri-spike-vxha) closes.
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

    /// `rlh` = "the value of the lh unit on the root element" (CSS Values 4
    /// §6.1.1) — a **tree-global** constant, unaffected by the consuming
    /// node's own font-size / line-height. The root element's own `1rlh`
    /// usage and a descendant's must agree on the same basis; this is the
    /// `root_line_height` / `used_line_height_length` derivation this issue
    /// added `ResolveContext::with_root_line_height` for (mirrors
    /// `rem_on_root_element_box_property_uses_own_font_size`'s `rem` pin).
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

    /// The default (initial) `line-height: normal` on the root propagates the
    /// same "no font metrics" wall through `rlh` to every descendant.
    #[test]
    fn rlh_falls_back_to_zero_when_root_line_height_is_normal() {
        let (root, child) = cascade_parent_child("html", None, "p", Some("padding: 1rlh"));
        assert_eq!(root.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.padding, Sides::all(ComputedLengthPercentage::Px(0.0)),);
    }

    /// `line-height: 1lh` is self-referential (CSS Values 4 §6.1.1, spec
    /// quote canonically documented on
    /// `crate::resolve::resolve_line_height`) — it must use the **parent's** // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    /// used line-height, not the declaring element's own font-size. The
    /// child's own font-size (50px) is deliberately different from the
    /// parent's (16px, initial) so a bug that leaks the child's own metrics
    /// in would be caught.
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

    /// When the parent's own line-height is unresolvable (`normal`), a
    /// child's self-referential `line-height: 1lh` falls back to
    /// `line-height`'s own initial value `normal` — not a fabricated length.
    #[test]
    fn line_height_lh_self_reference_falls_back_to_normal_when_parent_is_normal() {
        let (parent, child) = cascade_parent_child("div", None, "span", Some("line-height: 1lh"));
        assert_eq!(parent.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.line_height, ComputedLineHeight::Normal);
    }

    /// `line-height: 1rlh`, unlike `1lh` above, is **not** self-referential
    /// in this crate (bd raikiri-spike-vxha — `rlh`'s own definition, "the
    /// lh unit on the root element", is a tree-global constant that does not
    /// depend on the declaring element's position; see
    /// `crate::resolve::resolve_line_height`'s doc for why the literal // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    /// "Similarly, lh or rlh" spec wording is not followed for `rlh` on
    /// non-root elements). Three levels (root / middle / leaf) with
    /// **different** line-heights at the root and the immediate parent
    /// discriminate this: if `rlh` were (wrongly) treated as
    /// self-referential like `lh`, the leaf would pick up the *middle*
    /// element's used line-height (48px) instead of the root's (40px).
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

    /// The root element has no parent, so CSS Values 4 §6.1.1's "if the
    /// element has no parent" clause applies: the self-reference basis is
    /// the *initial* line-height, which is `normal` — always unresolvable.
    /// Mirrors `rem_on_root_element_resolves_against_initial_font_size` for
    /// `em`/`rem` on `font-size`.
    #[test]
    fn line_height_lh_self_reference_on_root_element_is_always_normal() {
        let cv = cascade_doc("", "html", Some("line-height: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    }

    // ── `font-size: 1lh` / `1rlh` (CSS Values 4 §6.1.1, bd raikiri-spike-yh3w) ──

    /// **The core bug this issue tracks.** Before the fix, `parse_font_size`
    /// dropped `font-size: 1lh` at *parse* time — not merely computing it
    /// wrong, but making the declaration invisible to cascade winner
    /// selection. `p { font-size: 1lh }` (specificity 0,0,1) must beat
    /// `* { font-size: 12px }` (specificity 0,0,0) under ordinary CSS
    /// cascade rules; before the fix, the `1lh` declaration was silently
    /// discarded and the lower-specificity `12px` declaration won by
    /// default (there being no competing candidate left).
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

    /// `font-size: 1lh` is self-referential (CSS Values 4 §6.1.1, "or font-*
    /// properties on the element they refer to") — it resolves against the
    /// **parent's** used line-height, mirroring `line-height: 1lh`'s own
    /// self-reference (`line_height_lh_self_reference_uses_parent_not_own_metrics`
    /// above).
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

    /// When the parent's own line-height is unresolvable (`normal`), a
    /// child's self-referential `font-size: 1lh` falls back to `font-size`'s
    /// own spec initial (`medium` = 16px) — not a fabricated ratio
    /// (cleanroom), and not `0px` either: unlike `border-width: 1lh` /
    /// `padding: 1lh` under `line-height: normal`
    /// (`lh_falls_back_to_zero_when_own_line_height_is_normal` above, which
    /// share a *generic* resolver with a known `0px` compromise tracked by
    /// bd raikiri-spike-k05m), `resolve_font_size` is a dedicated
    /// single-property resolver and falls back to its own true initial
    /// directly (`resolve_font_size` doc, "`lh` / `rlh` の自己参照" section).
    #[test]
    fn font_size_lh_falls_back_to_initial_when_parent_line_height_is_normal() {
        let (parent, child) = cascade_parent_child("div", None, "span", Some("font-size: 1lh"));
        assert_eq!(parent.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.font_size, ComputedLength(INITIAL_FONT_SIZE_PX));
    }

    /// The root element has no parent, so `font-size: 1lh` on the root
    /// itself is always unresolvable (CSS Values 4 §6.1.1 "if the element
    /// has no parent" → initial values → `line-height: normal`). Mirrors
    /// `line_height_lh_self_reference_on_root_element_is_always_normal`.
    /// Falls back to `font-size`'s own spec initial, same as the non-root
    /// case above.
    #[test]
    fn font_size_lh_on_root_element_falls_back_to_initial() {
        let cv = cascade_doc("", "html", Some("font-size: 1lh"));
        assert_eq!(cv.font_size, ComputedLength(INITIAL_FONT_SIZE_PX));
    }

    /// `font-size: 1rlh`, unlike `1lh` above, is **not** self-referential
    /// for a non-root element (same asymmetry as `line-height: 1rlh`,
    /// `line_height_rlh_in_line_height_uses_root_not_immediate_parent`
    /// above) — `rlh` always refers to the tree-global root line-height, not
    /// the immediate parent's. Three levels with **different** line-heights
    /// at the root and the immediate parent discriminate this: if `rlh`
    /// were (wrongly) treated as self-referential like `lh`, the leaf would
    /// pick up the *middle* element's used line-height (48px) instead of
    /// the root's (40px).
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

    /// Same 3-level tree as `font_size_rlh_uses_root_not_immediate_parent`,
    /// but with `1lh` on the leaf instead of `1rlh` — the two tests together
    /// discriminate a `Lh`/`Rlh` basis swap in `resolve_font_size` (48px vs
    /// 40px, the two possible wrong answers for each other's unit).
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

    /// Document 直下の **非 element** node は rem context を確定させない
    /// (`resolve_inheritance` の `child_ctx` の `None => None` arm)。
    ///
    /// `StyleDom::root_id` は Document node であって root element ではないので、
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
        assert_eq!(r.computed[a].font_weight, 700.0);
        assert_eq!(r.computed[b].font_weight, 900.0);
        assert_eq!(r.computed[c].font_weight, 900.0);
    }

    /// **D5 invariant** — `font-size` 版 (raikiri-spike-4rmu、`bolder` の
    /// `bolder_resolves_against_parent_computed_weight_through_staging` と同型)。
    ///
    /// `larger` / `smaller` は `SpecifiedValues` の staging 上で解決されるが、
    /// その基準は**親の computed font-size** でなければならない (CSS Fonts 4
    /// §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>)。
    /// `SpecifiedValues::inherit_from` が `font_size` を親からではなく
    /// `initial()` (16px) から seed すると 16 → 19.2 になり、この test だけが
    /// 落ちる (compile error にはならない)。
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

    /// 3 段の `larger` chain — staging を経ても compounding する
    /// (16 → 19.2 → 23.04)。`bolder_chain_compounds_through_staging` の
    /// font-size 版。
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

    /// root element の `font-size: larger` — 親が無いので initial (16px) 基準
    /// (`crate::specified::SpecifiedValues::finalize_as_root` doc の // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-o9h6)
    /// "if the element has no parent" 条項、SPEC-6 derivation と同じ pattern)。
    #[test]
    fn larger_on_root_element_resolves_against_initial_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: larger"));
        assert_eq!(cv.font_size, ComputedLength(19.2));
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

    // ── counter-* wire-through (CSS Lists 3 §4、raikiri-spike-s85 M5 pre-work) ──

    #[test]
    fn counter_reset_wired_through_cascade_from_inline_style() {
        // <div style="counter-reset: chapter"> → ComputedValues.counter_reset
        // に `[("chapter", 0)]` が届く。parser → PropertyValue → apply_value →
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
        // `[Literal("hello")]` が届く。parser → PropertyValue::Content →
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

    // ── string-set wire-through (CSS GCPM 3 §1.1.1、raikiri-spike-m5.3) ──

    #[test]
    fn string_set_wired_through_cascade_from_inline_style() {
        // <p style='string-set: chapter_title "hello"'> → ComputedValues.string_set
        // に `[(chapter_title, [Literal("hello")])]` が届く。
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

    // ── position: running() wire-through (CSS GCPM 3 §1.2.1、raikiri-spike-m5.4) ──

    #[test]
    fn running_template_wired_through_cascade_from_inline_style() {
        // <div style="position: running(header)"> → ComputedValues.running_templates
        // に `[RunningTemplate{name:"header"}]` が届く。parser → PropertyValue::Position
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
        // CSS GCPM 3 §1.2.1。position が non-inherited であることは CSS
        // Positioned Layout 3 §2 <https://www.w3.org/TR/css-position-3/#position-property>
        // の propdef "Inherited: no"。<div style="position: running(hdr)"> の
        // 子 <span> は自身の rule がなく running_templates は initial (empty)。
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
            "display must NOT inherit (CSS Display 3 §2 Inherited: no) — initial Inline"
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

    // ── direction wire-through (CSS Writing Modes 4 §2.1、raikiri-spike-l3wg) ──

    #[test]
    fn direction_wired_through_cascade_from_inline_style() {
        use crate::property::Direction;
        let cv = cascade_doc("", "p", Some("direction: rtl"));
        assert_eq!(cv.direction, Direction::Rtl);
    }

    #[test]
    fn direction_inherits_from_parent_element() {
        // CSS Writing Modes 4 §2.1: direction は **inherited**.
        use crate::property::Direction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Rtl);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].direction,
            Direction::Rtl,
            "child should inherit direction from parent (CSS Writing Modes 4 §2.1 Inherited: yes)"
        );
    }

    #[test]
    fn direction_child_own_value_wins_over_inherited() {
        use crate::property::Direction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl"));
        let span = doc.push_element(p, "span", Some("direction: ltr"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Rtl);
        assert_eq!(r.computed[span].direction, Direction::Ltr);
    }

    // ── text-align: match-parent (CSS Text 3 §6.1、raikiri-spike-l3wg) ──
    //
    // origin: raikiri-spike-ygl0 §8.2 spec lens F1 → raikiri-spike-l3wg. Before
    // this, `TextAlign::MatchParent` reached `ComputedValues.text_align`
    // unresolved (raikiri had no `direction` in the computed layer). These
    // tests exercise the *full* `cascade()` pipeline end-to-end, complementing
    // the `SpecifiedValues::finalize` unit tests in `specified.rs`.

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

    /// **The end-to-end pin for the whole `direction` + `text-align:
    /// match-parent` design.** The child declares *both* `direction: rtl`
    /// and `text-align: match-parent` on itself. Per CSS Text 3 §6.1
    /// `#valdef-text-align-match-parent` ("interpreted against **the
    /// parent's** direction value"), the resolution must use the parent's
    /// `ltr`, not the child's own `rtl`. This is the same invariant
    /// `specified::tests::finalize_match_parent_uses_parent_direction_not_own_direction_winner`
    /// pins at the `SpecifiedValues` unit level; this test additionally
    /// proves the wiring through `PropertyKey` winner selection and
    /// `apply_winners`' declaration-order walk, so a regression that
    /// resurfaces the same-node winner-order hazard through a different path
    /// (not just `SpecifiedValues::finalize`) would be caught here too.
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

    /// CSS Text 3 §6.1 verbatim: "Computes to start when specified on the
    /// root element." This is **not** the same rule as the parent-direction
    /// table — a root element declaring `direction: rtl` on itself must not
    /// affect its own `match-parent` resolution (there is no parent to
    /// consult at all).
    #[test]
    fn text_align_match_parent_on_root_element_resolves_to_start() {
        use crate::property::TextAlign;
        let cv = cascade_doc("", "html", Some("direction: rtl; text-align: match-parent"));
        assert_eq!(cv.text_align, TextAlign::Start);
    }

    /// Three-level chain: pins the induction that a computed `text_align` is
    /// never observed as `MatchParent` — the grandchild resolves against the
    /// *child's* already-resolved computed value (`Left`), not against
    /// `MatchParent` itself.
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
        // Verification #7 (bd 0vv.16): parent bold + child 未指定 = child 700。
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

    // ── font-weight bolder / lighter (CSS Fonts 4 §2.2、raikiri-spike-17s8) ──

    /// `<p style="font-weight: {parent}"><span style="font-weight: {child}">` を
    /// cascade して span の computed font-weight を返す。
    ///
    /// **親の computed value** を経由することが本 helper の主眼 — child は
    /// literal な spec 値ではなく、親が cascade を通して確定させた weight に
    /// 対して relative resolution される (bd 17s8 Verification #7)。
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
        // この 2 行は raikiri-spike-5iy が range を `[1,1000]` に広げて初めて
        // author から到達可能になったため、bundle 固有の regression guard。
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

    /// `resolve_relative_weight` の doc 「非有限 `inherited` — 本関数は
    /// guard しない」節が記述する非対称処理を pin する (bd
    /// raikiri-spike-sxd7)。
    ///
    /// bd raikiri-spike-kfl7 precedent (「guard は sink 境界に置く、resolve
    /// 層には置かない」) に従い、**本関数自体は変更しない** — 非対称は
    /// バグとして修正されるものではなく、非有限 `inherited` (通常経路では
    /// 型/parse guard により到達しないが `ComputedValues` の直接構築からは
    /// 到達しうる) に対する現状の table 分岐の帰結として、以降の regression
    /// で挙動が変わらないことを保証するために pin する。sink 側の guard は
    /// `crates/raikiri-dom/src/layout.rs` の `sanitize_font_weight`
    /// (`preshape_text` が `parley::FontWeight::new` に渡す直前) に別途ある。
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

    /// `resolve_relative_font_size` を unit 関数として直接叩く — cascade
    /// harness に依存せず ratio (1.2) の適用を検証する
    /// (`font_weight_bolder_lighter_table_all_six_rows` の font-size 版、
    /// raikiri-spike-4rmu)。
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

    /// `resolve_against_inherited` の戻り値 `ResolvedAgainstInherited` が
    /// 中身を無損失で運ぶこと — 型を足したことで解決結果そのものが変わって
    /// いないことの pin (bd raikiri-spike-7m33)。`as_property_value` (覗き見)
    /// と `into_property_value` (消費) の両方を、resolve 対象・pass-through
    /// 対象の 2 パターンで確認する。
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
    }

    #[test]
    fn font_weight_bolder_wired_through_cascade_from_parent_computed() {
        // bd 17s8 Verification #3 / #4 / #5。親の **computed** weight に対して
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
        // bd 17s8 Verification #6。
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
        // bd 17s8 Verification #7 の核心。親の declaration は `bold` keyword
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
        // raikiri-spike-5iy: spec range `[1,1000]` の両端が cascade まで届く。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1")).font_weight,
            1.0
        );
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1000")).font_weight,
            1000.0
        );
        // fractional は f32 格上げ以降、丸めずそのまま computed value まで届く
        // (bd raikiri-spike-e52s — 旧実装は round-half-away-from-zero で 101 に
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
        // computed side で pin する (parse 側の同値 pin は
        // `crate::property::tests::font_weight_wpt_font_weight_computed_150_25`)。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 150.25")).font_weight,
            150.25
        );
    }

    #[test]
    fn bolder_lighter_resolve_against_unrounded_fractional_parent_weight() {
        // bd raikiri-spike-e52s の origin failure scenario (3 件、issue 本文の
        // "Failure scenario" 節をそのまま pin)。丸めが `u16` computed 表現に
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

    /// counter-* は non-inherited — 親 element に counter 値があっても child は
    /// inherit_from で shared empty Arc slot に落ちる。この pin が「Arc wrap 単独
    /// (short-circuit 無し) でも child stack entry の parent Arc bump が即 empty
    /// slot に置換される」ことを保証する。d9y.2 short-circuit 不採用の正当化
    /// assertion。
    ///
    /// CSS Lists 3 §4: 3 property とも property table が `Inherited: no`。
    /// See <https://www.w3.org/TR/css-lists-3/#auto-numbering>.
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

    // ── font-family per-node malloc regression (raikiri-spike-no7b) ──
    //
    // reviewer:perf finding (origin: raikiri-spike-zpui §8.2、out-of-diff
    // pre-existing): `ComputedValues.font_family` は本 crate で最後に
    // Arc-share されていなかった `Vec` payload (d9y.1 が Content/StringSet、
    // d9y.2 が counter-* を対応済み)。あちら (non-inherited) と違い
    // `font-family` は **inherited** なので、支配的な per-node cost は
    // 「毎 node で initial にリセットする」コストではなく inheritance walk
    // (`SpecifiedValues::inherit_from` の `parent.font_family.clone()`) の
    // コスト — 同じ `Arc` fix でもコストの形が違う。d9y.1/d9y.2 と同じ
    // behavioral-proxy methodology: `heap` 計測は allocator hook 依存なので、
    // `Arc::ptr_eq` を deep-clone regression の canary として使う。

    /// **独立した** 2 回の `cascade()` 呼び出し (別々の `TestDoc`、共有する
    /// inheritance 祖先なし) でも、root の computed value は**同一**の
    /// `initial_font_family()` Arc slot を指さなければならない。
    ///
    /// この test は「同一 document 内の 2 sibling element」ではなく
    /// **document をまたぐ** 形でなければならない — 単一 document 内では
    /// `font-family` が **inherited** なので、rule 無しの sibling 2 つは
    /// `SpecifiedValues::inherit_from` の「親の Arc をコピーする」経路で
    /// 既に font_family Arc を共有してしまい、`initial_font_family()` 自体は
    /// その walk を seed するために `cascade()` 1 回あたり高々 1 回しか
    /// 呼ばれない。そのため同一 document 版の test では、
    /// `initial_font_family()` 内の `OnceLock` を削除しても green のまま
    /// になってしまう (raikiri-spike-no7b 実装時に perturbation で確認済み —
    /// 同一 document 形は下記
    /// `resolve_inheritance_shares_font_family_arc_from_parent_when_child_has_no_declaration`
    /// を再度 test しているだけで、shared slot 自体は pin していない)。
    /// 単一 call site での直接 pin は
    /// `property::tests::initial_font_family_shares_arc_slot_across_calls`
    /// を参照。本 test はそれに加えて、slot が別々の `cascade()` 実行間
    /// (例: 1 process 内での複数 page rendering) をまたいで生存することを
    /// 確認する — 意図は `initial_empty_content_and_string_set_share_arc_slot`
    /// と同じだが、対象は非空の initial 値 (`[Atom::from("serif")]`) であり、
    /// この field の inherited 性質に合わせて adapt してある。
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
             slot across independent cascade() runs (raikiri-spike-no7b)"
        );
    }

    /// `* { font-family: ... }` × N element で、matching 全 element の
    /// `Vec` は勝者 declaration の Arc を共有しなければならない — mirrors
    /// `cascade_shares_content_arc_across_universal_selector_matches` (d9y.1)。
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
            "cascade must Arc-share font_family across universal-selector matches \
             (raikiri-spike-no7b)"
        );
    }

    /// `font-family` declaration を持たない child は、親と**同一**の Arc を
    /// (`Arc::ptr_eq`) 継承しなければならない — 中身を再 clone したものでは
    /// ならない。これは bd issue がこの (inherited) field の支配的コストと
    /// 名指しした inheritance-walk cost そのものであり、d9y.2 の
    /// non-inherited「shared empty slot にリセットする」形とは異なる。
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
             Arc by identity, not a re-cloned Vec (raikiri-spike-no7b)"
        );
    }

    // ── padding wire-through (CSS Box 3 §4.1 + §4.2、raikiri-spike-0vv.6) ──
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
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で pin: `padding-top: 5px; padding: 10px;`
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
        // 逆順で pin: `margin-top: 10px; margin: 0px;`
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

    // ── <img width>/<img height> presentational hint (bd raikiri-spike-5z86.7,
    // HTML LS https://html.spec.whatwg.org/multipage/rendering.html#dimRendering) ──

    #[test]
    fn img_width_and_height_attributes_promoted_to_computed_style() {
        // Acceptance (bd raikiri-spike-5z86.7): `<img width="100"
        // height="50">` の HTML attribute が author CSS 無しでも computed
        // width/height に届く。
        let mut doc = TestDoc::new();
        let img =
            doc.push_element_with_attrs(0, "img", None, &[("width", "100"), ("height", "50")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(
            r.computed[img].height,
            ComputedLengthPercentageOrAuto::Px(50.0)
        );
    }

    #[test]
    fn img_width_attribute_alone_does_not_set_height() {
        // 独立 mapping — `width` だけ指定した場合 `height` は initial のまま
        // (spec は 2 属性を "respectively" と個別に mapping する)。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(0, "img", None, &[("width", "100")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(r.computed[img].height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn img_width_attribute_zero_is_a_valid_hint() {
        // 「maps to the dimension property」であり「…(ignoring zero)」では
        // ないことの pin (cf. `<table width>` は ignoring-zero) — `width="0"`
        // は 0px という有効な hint になる。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(0, "img", None, &[("width", "0")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn img_width_attribute_percentage_and_decimal_accepted() {
        // bd raikiri-spike-5z86.7 dispatch prompt は "非負整数" と要約したが、
        // spec 本文の "rules for parsing dimension values" は percentage /
        // 小数も受理する — 実装はその本文どおり (`parse_html_dimension_value`
        // doc 参照)。
        let mut doc = TestDoc::new();
        let pct = doc.push_element_with_attrs(0, "img", None, &[("width", "50%")]);
        let dec = doc.push_element_with_attrs(0, "img", None, &[("width", "10.5")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[pct].width,
            ComputedLengthPercentageOrAuto::Percent(50.0)
        );
        assert_eq!(
            r.computed[dec].width,
            ComputedLengthPercentageOrAuto::Px(10.5)
        );
    }

    #[test]
    fn img_width_attribute_trailing_garbage_does_not_fail_parse() {
        // legacy dimension-value microsyntax の寛容さ pin: 数値直後の garbage
        // は失敗にならない (`"42px"` → 42px, naive integer parse ならここで
        // 失敗していたはず)。先頭 whitespace の skip も同時に確認。
        let mut doc = TestDoc::new();
        let px = doc.push_element_with_attrs(0, "img", None, &[("width", "  42px")]);
        let pct_dot = doc.push_element_with_attrs(0, "img", None, &[("width", "10.%")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[px].width,
            ComputedLengthPercentageOrAuto::Px(42.0)
        );
        // cov:ignore: the message-format branch of this `assert_eq!` only
        // executes on failure; this assertion passes on every run, so
        // llvm-cov reports the message-string line as an uncovered added
        // line even though the assertion itself runs (same shape as
        // `counter_style.rs`'s `reserved_rule_names_are_dropped` cov:ignore).
        assert_eq!(
            r.computed[pct_dot].width,
            ComputedLengthPercentageOrAuto::Percent(10.0),
            "trailing `.` with no fractional digit still checks the following `%`"
        );
    }

    #[test]
    fn img_width_attribute_invalid_or_negative_value_produces_no_hint() {
        // 失敗 (parse failure) は「hint を作らない」に落ちる — 属性が無いのと
        // 同じ扱いで width は initial `auto` のまま。`-5` は algorithm に
        // `-`/`+` 分岐が無いため即失敗 (先頭が ASCII digit でない)。
        let mut doc = TestDoc::new();
        let garbage = doc.push_element_with_attrs(0, "img", None, &[("width", "abc")]);
        let negative = doc.push_element_with_attrs(0, "img", None, &[("width", "-5")]);
        let empty = doc.push_element_with_attrs(0, "img", None, &[("width", "")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[garbage].width,
            ComputedLengthPercentageOrAuto::Auto
        );
        assert_eq!(
            r.computed[negative].width,
            ComputedLengthPercentageOrAuto::Auto
        );
        assert_eq!(
            r.computed[empty].width,
            ComputedLengthPercentageOrAuto::Auto
        );
        // Independently pin the *other* rejection layer for the empty-
        // string case: `TestElementRef::attr()` itself normalises `""` to
        // `None` (matching the `StyleElement::attr` trait contract and the
        // real `ElementRef::attr()`), so `push_img_dimension_hints` never
        // even calls `parse_html_dimension_value` for `width=""` — the
        // `Auto` result above isn't (only) a parse-failure outcome.
        let node = doc
            .node(StyleNodeId::new(empty as u64))
            .expect("node exists");
        let elem = node.as_element().expect("element node");
        assert_eq!(elem.attr("width"), None);
    }

    #[test]
    fn non_img_element_width_height_attributes_not_promoted() {
        // bd raikiri-spike-5z86.7 scope narrowing: mapping は `img` のみ。
        // 同じ attribute を持つ `div` は影響を受けない。
        let mut doc = TestDoc::new();
        let div =
            doc.push_element_with_attrs(0, "div", None, &[("width", "100"), ("height", "50")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(r.computed[div].height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn img_width_attribute_overridable_by_inline_author_style() {
        // Cascade-origin pin: presentational hint と inline style は共に
        // `Origin::Author` (`push_img_dimension_hints` doc の "Cascade
        // origin" 節) なので origin では決着せず、同一 origin 内の
        // tie-break (specificity) に落ちる — inline style の
        // `INLINE_SPECIFICITY` (`1 << 30`) は hint の `specificity = 0`
        // (`PRESENTATIONAL_HINT_SPECIFICITY`) より圧倒的に大きいので、
        // exact-tie の心配なく無条件に勝つ (対照的に、下の
        // `..._by_author_stylesheet_regardless_of_specificity` は
        // 両者とも specificity 0 になり得るので exact-tie 経路を通る)。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(
            0,
            "img",
            Some("width: 50px"),
            &[("width", "100"), ("height", "100")],
        );
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: message-format branch of this `assert_eq!` only
        // executes on failure (see the `pct_dot` cov:ignore above for the
        // full explanation of this line-coverage false positive).
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(50.0),
            "author inline style must override the HTML presentational hint"
        );
        // cov:ignore: same false positive, second assertion in this test.
        assert_eq!(
            r.computed[img].height,
            ComputedLengthPercentageOrAuto::Px(100.0),
            "height has no author override, so the hint still applies"
        );
    }

    #[test]
    fn img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity() {
        // Exact-tie case pin (`push_img_dimension_hints` doc's "Cascade
        // origin" §2): the hint and this `* { width: 30px }` rule are both
        // `Origin::Author`, both specificity 0 (universal selector), and
        // both `source_order = 0` (first/only rule in an otherwise-empty
        // `RuleTree`) — an exact 3-tuple tie. `beats` resolves the tie in
        // favor of whichever candidate is scanned later in `pick_winners`,
        // and `collect_cascaded` pushes the hint *before* stylesheet-rule
        // matching for the same element, so the real rule wins. This is not
        // a specificity-driven outcome (both sides are 0) — it pins the
        // push-order invariant instead.
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, "* { width: 30px }");
        let img = doc.push_element_with_attrs(0, "img", None, &[("width", "100")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: same false positive as the other `img_width_attribute_*`
        // tests above (message-format branch of `assert_eq!` only executes
        // on failure).
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(30.0),
            "real author rule must win the exact-tie via push-order (hint pushed first)"
        );
    }

    #[test]
    fn img_tag_name_match_is_ascii_case_insensitive() {
        // `push_img_dimension_hints` 自身の `elem.tag_name().eq_ignore_ascii_case`
        // 判定を確認 — `match_simple_selectors` の `Component::LocalName` 判定
        // (bd raikiri-spike-flln.1、旧名 `match_by_tag`) と同じ寛容さの、独立
        // した別実装。real DOM (html5ever) は tag name を常に lowercase に
        // 正規化するので実運用では観測されないが、`StyleElement` は特定 DOM
        // 実装に紐付かない generic trait なので defensive に確認しておく。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(0, "IMG", None, &[("width", "100")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
    }

    #[test]
    fn foreign_namespace_img_local_name_does_not_get_the_hint() {
        // Codex §8.3 final-review finding 1 (2026-08-11): the mapping is
        // HTML-namespace-specific (`push_img_dimension_hints` doc's
        // namespace-gate comment). A foreign-namespace element that merely
        // shares the local name "img" must not pick up the presentational
        // hint, even though `tag_name()` alone would match.
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_namespace(
            0,
            "img",
            "http://example.com/not-html",
            &[("width", "100")],
        );
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: message-format branch of this `assert_eq!` only
        // executes on failure (same false positive as the other
        // `img_width_attribute_*` tests above).
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Auto,
            "foreign-namespace element sharing the \"img\" local name must not get the hint"
        );
    }

    #[test]
    fn test_dom_attr_style_matches_inline_style_source_contract() {
        // `push_img_dimension_hints` は `elem.attr("width")`/`attr("height")`
        // 経由で `TestElementRef::attr()` の override (`crate::test_dom`,
        // bd raikiri-spike-5z86.7 で追加) を叩く。`StyleElement::attr` の
        // trait doc ("Default handles `style` by delegating to
        // `inline_style_source`; overrides must preserve that contract")
        // をこの override が守っていることを直接確認する — real DOM
        // (`crates/raikiri-dom/src/dom_impl.rs`) の `ElementRef::attr()` も
        // 同じ `"style"` 特別扱いを持つので、ここが崩れると実 DOM との
        // 挙動差が生まれる。
        let mut doc = TestDoc::new();
        let id = doc.push_element(0, "div", Some("color: red"));
        let node = doc.node(StyleNodeId::new(id as u64)).expect("node exists");
        let elem = node.as_element().expect("element node");
        assert_eq!(elem.attr("style"), Some("color: red"));
        assert_eq!(elem.attr("style"), elem.inline_style_source());
    }

    #[test]
    fn parse_html_dimension_value_matches_spec_algorithm_directly() {
        // `parse_html_dimension_value` の unit-level pin — 上の end-to-end
        // test 群と違い、cascade を経由せず algorithm 自体の分岐を直接叩く。
        assert_eq!(parse_html_dimension_value("100"), Some(Length::Px(100.0)));
        assert_eq!(parse_html_dimension_value("0"), Some(Length::Px(0.0)));
        assert_eq!(
            parse_html_dimension_value("50%"),
            Some(Length::Percent(50.0))
        );
        assert_eq!(parse_html_dimension_value("10.5"), Some(Length::Px(10.5)));
        assert_eq!(
            parse_html_dimension_value("10.5%"),
            Some(Length::Percent(10.5))
        );
        // 2 桁以上の小数部 — fractional-digit loop が 1 回で `break` せず
        // 「まだ digit が続く」経路 (`parse_html_dimension_value` 内 `loop`
        // の non-break iteration) を通ることを pin。1 桁小数
        // (上の `10.5` / `10.5%`) だけでは exercise されない分岐。
        assert_eq!(
            parse_html_dimension_value("12.345"),
            Some(Length::Px(12.345))
        );
        // cov:ignore: message-format branch of this `assert_eq!` only
        // executes on failure (see `img_width_attribute_trailing_garbage_
        // does_not_fail_parse`'s `pct_dot` cov:ignore for the full
        // explanation of this line-coverage false positive).
        assert_eq!(
            parse_html_dimension_value("  42px"),
            Some(Length::Px(42.0)),
            "leading whitespace skipped, trailing garbage after the number ignored"
        );
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("10.%"),
            Some(Length::Percent(10.0)),
            "trailing `.` with no fractional digit still advances past it before the % check"
        );
        assert_eq!(parse_html_dimension_value("abc"), None);
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("-5"),
            None,
            "no sign branch in the algorithm"
        );
        assert_eq!(parse_html_dimension_value(""), None);
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("   "),
            None,
            "whitespace-only input never reaches a digit"
        );
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("."),
            None,
            "a lone `.` is not a leading digit"
        );
    }

    #[test]
    fn apply_value_direct_margin_shorthand_fall_through() {
        // `apply_value` の `PropertyValue::Margin(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 4 longhand に展開する)。**これは
        // safety net ではない** (bd raikiri-spike-8kn8 で framing 訂正) — 万一
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

    // ── border longhand + shorthand cascade (raikiri-spike-0vv.12) ──

    #[test]
    fn border_shorthand_then_longhand_later_longhand_wins() {
        // spec (CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort>): 同一 declaration
        // block 内で shorthand + longhand が declared された場合、後方
        // declaration が同 rank/spec/order で勝つ。
        // `border: 1px solid red; border-top-width: 10px;` →
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
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で pin: `border-top-width: 10px; border: 1px solid red;`
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
        // 37n sibling: margin / padding non-inherited test
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
    fn apply_value_direct_border_shorthand_fall_through() {
        // `apply_value` の `PropertyValue::Border(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 12 longhand に展開する)。**これは
        // safety net ではない** (bd raikiri-spike-8kn8 で framing 訂正、margin
        // fall-through と対称) — 万一 regression / bypass 経路で到達すると
        // `target.border = sides` の atomic 上書きが 12 longhand winner を必ず
        // 破壊する。到達した時点で既に bug であり、本 test は arm を直接叩いて
        // `unreachable!` 化 or 空 arm regression を捕捉する canary。
        let mut cv = SpecifiedValues::initial();
        // raikiri-spike-0vv.17: `BorderColor::CurrentColor` を明示 fixture 化
        // (fall-through arm は payload の shape を保持することを pin する)。
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
    // `add_stylesheet` の**後**に declaration を shorthand variant へ書き戻す
    // post-parse mutation 経路 — `crate::rule::parse_declaration_block` の
    // parse-time 展開はこの経路を守らない (`declaration_block_never_emits_
    // shorthand_keys` は parse 出口しか見ない)。bd raikiri-spike-qzn3 以降この
    // 経路は crate 内からのみ到達可能なので `collect_cascaded` 入口の展開は
    // crate 内 invariant guard である。根拠は
    // `crate::rule::expand_shorthand_into` doc が canonical。
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
    /// `css` を `RuleTree::add_stylesheet` で parse したあと、
    /// `style_rules[0].declarations[idx].value` を `injected` に差し替え、
    /// `<div>` 1 個の document に cascade して computed value を返す。
    ///
    /// この手つきは **nqkj が報告した当時の Consumer 経路**そのもの。qzn3 で
    /// 3 field を `pub(crate)` に絞ったので、もう crate 外からは書けない。
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
        // to be !important.") が element cascade 入口の展開でも保たれること。
        //
        // 注入した shorthand は `!important` を継承する (`[0]` は `!important`
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
}
