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
use selectors::parser::{Combinator, NthSelectorData, Selector, SelectorIter, SelectorList};

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
use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};
use crate::{Direction, RaikiriSelectorImpl};

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
/// 同 hint の source_order。`0` — 実 stylesheet の最初の rule
/// ([`crate::ruletree::RuleTree::add_stylesheet`] は空 `RuleTree` への
/// 最初の rule に `source_order = 0` を採番する) と数値上 tie し得る値だが、
/// bd raikiri-spike-wo36 で hint 専用の `cascade_rank` tier
/// ([`Origin::AuthorPresentationalHint`]) を導入して以降、この tie は
/// 実際には発生しない — rank 差が specificity/source_order より先に
/// tuple compare で決着するため ([`push_img_dimension_hints`] doc の
/// "Cascade origin" 節参照)。`0` という値自体は「他候補と衝突しない値」を
/// 意図したものではなく、単に real stylesheet rule の source_order と同じ
/// 値域を使うという単純さのための選択。
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

/// Cascade origin + `!important` flag に基づく優先度 rank (raikiri-spike-m1.22、
/// 3rd origin tier は bd raikiri-spike-wo36、4th origin tier ([`Origin::User`])
/// は bd raikiri-spike-pdta — pdta で 4-tier 全体を re-derive した、単純な
/// 番号ずらしではない点に注意、下記参照)。
///
/// 高いほど勝つ。CSS Cascading L4 §6.1 "Cascade Sorting Order"
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の Origin and Importance
/// 段を表現する (origin の定義は §6.2
/// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>、`!important` に
/// よる反転は §6.3 <https://www.w3.org/TR/css-cascade-4/#importance>)。
///
/// # UA / User / Author の 3-origin 部分 (spec-verbatim)
///
/// §6.1 "Cascade Sorting Order" は origin 優先度を降順 8 項目のリストで
/// 定める (2026-08-12 再 fetch 確認、transition/animation の 2 項目は本
/// crate 未実装のため以下では省略):
/// - "Important user agent declarations"
/// - "Important user declarations"
/// - "Important author declarations"
/// - "Normal author declarations"
/// - "Normal user declarations"
/// - "Normal user agent declarations"
///
/// "Declarations from origins earlier in this list win over declarations
/// from later origins" — すなわち降順优先度を昇順 (弱い→強い) rank に
/// 反転すると: `Normal UA < Normal User < Normal Author < Important Author
/// < Important User < Important UA`。UA/User/Author の 3-origin に関しては
/// これが spec の直接記述であり、Important 段の User の位置 (`Important
/// Author < Important User < Important UA`) も含めて **verbatim** — 導出
/// ではない。
///
/// # [`Origin::AuthorPresentationalHint`] の挿入 (CSS Cascading L5 §6.5)
///
/// [`Origin::AuthorPresentationalHint`] は CSS Cascading L5 §6.5
/// "Precedence of Non-CSS Presentational Hints"
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>, 2026-08-12 再 fetch
/// 確認) が定める "author presentational hint origin"。L5 自身の §6.1 も
/// 上記と同じ 8 項目リストのままで hint origin をそこに明示的には
/// 挿入しない (2026-08-12 再 fetch で確認 — hint の位置付けは §6.5 側の
/// 独立した記述に委ねられている)。以下は §6.5 の段落から**関連する抜粋を
/// verbatim 引用**したもの (段落全体の逐語コピーではない — 完全性を主張
/// しない):
/// - "All document language-based styling must be translated to
///   corresponding CSS rules and enter the cascade as rules in **either
///   the UA-origin or** a special-purpose author presentational hint
///   origin between the regular user origin and the author origin" —
///   "treated as an independent origin"
/// - "Presentational hints entering the cascade as author presentational
///   hint origin rules can be overridden by author-origin styles, but not
///   by non-important user-origin styles"
/// - "**A document language may define whether** such a presentational
///   hint enters the cascade as UA-origin or author-origin; if so, the UA
///   must behave accordingly. For example, SVG maps its presentation
///   attributes into the author origin."
/// - "however for the purpose of the revert keyword (but not for the
///   revert-layer keyword) it is considered part of the author origin"
///
/// **どの tier を選ぶかは host language 次第、という点に注意**: 上記 3 番目の
/// 引用の通り、spec は presentational hint の origin 配置を host language に
/// 委ねる (UA-origin か author-origin かを明示的に選べる、SVG は author
/// origin を選ぶ例)。HTML Living Standard §15.2 は自身のこの選択を
/// "author-level zero-specificity presentational hints" と呼ぶだけで、
/// CSS-Cascade-5 の origin taxonomy の用語 ("UA-origin" / "author-origin" /
/// "author presentational hint origin") を一切参照しない — つまり HTML LS
/// 自身は「UA-origin」「author-origin」「author presentational hint
/// origin」のどれを選んだとも明言していない。`<img>` の width/height hint を
/// 独立 origin tier ([`Origin::AuthorPresentationalHint`]) に置くという本
/// crate の判断 ([`push_img_dimension_hints`] doc 参照) は、したがって
/// **spec が直接指定する結論ではなく**、"author-level" という HTML LS の
/// 言葉遣いと上記引用群を突き合わせた this crate の解釈 (bd
/// raikiri-spike-5z86.7 / bd raikiri-spike-wo36 で reviewer:spec が
/// defensible と判定済みの judgment call)。
///
/// 上記 §6.5 の引用が直接定めるのは **Normal 段の位置**だけ: hint は
/// "between the regular user origin and the author origin" — Normal User
/// と Normal Author の間。Important 段での `AuthorPresentationalHint` の
/// 位置は spec に verbatim 記述が無い — presentational hint は host
/// language 側 (HTML LS §15.2) が生成するもので常に non-important なため、
/// spec 側にも important-hint tier を定める動機が無い。この不在を、
/// origin 独立性 (「独立 origin である」という上記引用) と §6.1 の
/// **reversal-equivalence** から導出する: UA/User/Author の 3-origin に
/// 限れば、Important 段の昇順順序 (`Author < User < UA`) は Normal 段の
/// 昇順順序 (`UA < User < Author`) のちょうど逆順になっている — これは
/// 類推ではなく、§6.1 の同じ 8 項目リストから直接読み取れる事実 (上記
/// 「UA / User / Author の 3-origin 部分」節参照)。hint (4th origin) に
/// この同じ reversal 機構をそのまま延長するのが、追加の仮定を要さない
/// 最小の拡張であり、それが以下の順序を導く: Normal 段で hint が User と
/// Author の間に挟まる (「独立した第三者」として) のと同じ相対位置を、
/// reversal された Important 段 (Important Author と Important User の
/// 間) でも保つ、という位置取りを採用する (verbatim ではない、derived —
/// ただし spec 自身の reversal 機構をそのまま延長しただけで、恣意的な
/// 選択の余地は無い)。
///
/// # 4-tier 全体の rank 表 (pdta の re-derivation)
///
/// 上記 2 節を合成すると、Normal / Important 各 4 origin の順序は:
/// - Normal   : UA < User < AuthorPresentationalHint < Author (spec-verbatim
///   な UA/User/Author の骨格に、spec-verbatim な hint の位置 — User と
///   Author の間 — を挿入)
/// - Important: Author < AuthorPresentationalHint < User < UA (反転した
///   spec-verbatim な Author/User/UA の骨格に、derived な hint の位置 —
///   Important Author と Important User の間 — を挿入)
///
/// rank 番号を割り当てると (0 が最弱、7 が最強):
///
/// | origin                     | Normal | Important |
/// |-----------------------------|--------|-----------|
/// | `UserAgent`                 | 0      | 7         |
/// | `User`                      | 1      | 6         |
/// | `AuthorPresentationalHint`  | 2      | 5         |
/// | `Author`                    | 3      | 4         |
///
/// 検算: `min(Important) = 4 > max(Normal) = 3` — 「any important
/// declaration beats any normal declaration」(§6.1/§6.3) を満たす。UA/User/
/// Author の 3 列はいずれも Normal 昇順・Important 降順が spec-verbatim の
/// 骨格 (`0,1,3` と `7,6,4`) と一致し、hint の挿入 (`2` / `5`) は両列とも
/// 「User と Author の間」という同じ relative position を保つ (対称)。
///
/// `(AuthorPresentationalHint, true)` の arm は現状
/// [`push_img_dimension_hints`] から到達しない (常に `important = false` で
/// push する)、`(User, false)` / `(User, true)` の 2 arm は現状どの
/// production 呼び出し元からも到達しない ([`Origin::User`] の doc の
/// "no production producer" 節参照、bd raikiri-spike-d7h3 が producer 追加を
/// 追跡) — [`crate::page::cascade_page`] も同じ [`Origin`] を経由するため、
/// これらも `unreachable!()` にはせず total function として値を返す。
///
/// `revert` keyword carve-out (上記 4 番目の引用: "it is considered part of
/// the author origin" — `revert-layer` は対象外) は本 crate に現状影響しない
/// — `revert`/`revert-layer` CSS-wide keyword 自体がまだ未実装
/// ([`crate::property`] の "CSS-wide keyword (canonical)" 節、"Epic 7"
/// milestone 参照)。実装時にこの carve-out の special-case が必要になる。
///
/// `@page` cascade (raikiri-spike-m4.1) も同じ origin ordering を共有するため
/// `pub(crate)` で公開し [`crate::page::cascade_page`] から reuse。
pub(crate) fn cascade_rank(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::User, false) => 1,
        (Origin::AuthorPresentationalHint, false) => 2,
        (Origin::Author, false) => 3,
        (Origin::Author, true) => 4,
        (Origin::AuthorPresentationalHint, true) => 5,
        (Origin::User, true) => 6,
        (Origin::UserAgent, true) => 7,
    }
}

/// `collect_cascaded` は DFS で node を訪れる。bd raikiri-spike-flln.1 以前
/// (combinator 非対応) は「per-node の処理は他の node の状態に依存しないため
/// 訪問順は無関係」だった。descendant/child combinator (bd
/// raikiri-spike-flln.2) の追加でこの前提は**もう成り立たない** — 各 element
/// の selector matching は本関数 local の `ancestor_path`（「これまでに
/// 訪れた祖先 element の id 列」）を参照するため、**祖先を子孫より先に処理する
/// pre-order 訪問が正しさの前提**になった (祖先が先に積まれていなければ
/// descendant/child の ancestor 参照が空振りする)。overflow 回避のため
/// explicit `Vec` stack で iterative に書く方針 (roborev job 199) 自体は
/// 変わらないが、stack の要素は素の `StyleNodeId` ではなく `(StyleNodeId,
/// usize)` — 後者は「この node を処理する直前に ancestor path を truncate
/// すべき長さ」。詳細は本関数の実装コメント参照。
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
    // Document-wide constant (bd raikiri-spike-tqwi) — read once rather than
    // per (node, rule) pair inside the loop below.
    let quirks_mode = dom.quirks_mode();
    // Stack entries pair a node id with the `ancestor_path` length it should
    // be truncated to *before* that node is processed (bd
    // raikiri-spike-flln.2). `stack` itself interleaves the pending work of
    // multiple subtrees in one flat `Vec` (sibling branches, cousins, ...),
    // so a plain push/pop can't recover "the current node's actual ancestor
    // chain" by itself — truncating `ancestor_path` to the depth recorded
    // when each entry was pushed undoes whatever a since-fully-processed
    // sibling subtree appended, reconstructing exactly the root..parent
    // chain for whichever node is popped next. Standard technique for
    // recovering DFS ancestor paths from a single explicit stack; it stays
    // O(1) amortized (`Vec::truncate` just shrinks `len`, no deallocation)
    // and needs no `HashMap`/parent-pointer side table.
    let mut stack: Vec<(StyleNodeId, usize)> = vec![(id, 0)];
    // Ancestor **element** ids, root-most first / immediate-parent last
    // (`ancestor_path.last()` = current node's parent). Only `Element`-kind
    // nodes are ever pushed — `Document`/`Text`/etc. can never be matched by
    // a compound selector, so they must not count as a combinator ancestor
    // either (CSS Selectors L4 descendant/child combinators are defined in
    // terms of element ancestry, e.g.
    // <https://www.w3.org/TR/selectors-4/#descendant-combinators> "an
    // element B that is an arbitrary descendant of some ancestor element
    // A" — verbatim (see `match_combinator_chain`'s "Spec provenance note"
    // for how this text was confirmed), both sides are elements).
    let mut ancestor_path: Vec<StyleNodeId> = Vec::new();
    while let Some((id, depth)) = stack.pop() {
        ancestor_path.truncate(depth);
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
                // HTML presentational hints (bd raikiri-spike-5z86.7,
                // retagged to `Origin::AuthorPresentationalHint` by bd
                // raikiri-spike-wo36). This push is kept ahead of
                // stylesheet-rule matching / inline style below for
                // historical/document-order reasons, but it is no longer a
                // *correctness* requirement: since the hint has its own
                // `cascade_rank` tier (strictly between `UserAgent` and
                // `Author`, see `push_img_dimension_hints` doc's "Cascade
                // origin" section), rank alone decides against any real
                // Author-origin declaration regardless of specificity,
                // source_order, or push order — no tie can occur (that was
                // only possible before wo36, when hint and real Author
                // declarations shared the same `Origin::Author` rank).
                // Re-verified after bd raikiri-spike-pdta inserted the 4th
                // `Origin::User` tier: the hint's `cascade_rank` value moved
                // (see `cascade_rank`'s rank table) but stayed strictly
                // between `Origin::User` and `Origin::Author` — never equal
                // to the real `Author` rank in either the Normal or the
                // Important half of the table — so this reasoning still
                // holds unchanged; no test pins the push order itself
                // (nothing here is order-*dependent* left to pin), but
                // `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
                // continues to pin the outcome this comment claims.
                push_img_dimension_hints(&elem, &mut out.decls);
                // stylesheet rule matching
                for rule in &rule_tree.style_rules {
                    if let Some(spec) = match_complex_selector_list(
                        &rule.selectors,
                        dom,
                        &elem,
                        id,
                        &ancestor_path,
                        quirks_mode,
                    ) {
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
                // This element becomes an ancestor for its own children
                // (pushed just below with `ancestor_path.len()` as their
                // truncation depth) — bd raikiri-spike-flln.2.
                ancestor_path.push(id);
            }
            // stack は LIFO なので document order で push するため reverse。
            // `child_ids` イテレータを直接 `stack` へ `extend` し、今回追加した
            // 末尾スライスだけを in-place `reverse()` する — 都度捨てる中間
            // `Vec` を経由しない (bd raikiri-spike-75ch)。`stack` 自体の
            // capacity growth は元の `for .. { stack.push(..) }` と同じ
            // amortized pattern のままで、ここで削れるのは「今回だけの捨て
            // Vec」1 本分のみ。
            //
            // なぜ document order (pre-order) を保つ**必要がある**か (bd
            // raikiri-spike-flln.2 でここの結論が反転): 本関数冒頭のコメント
            // の通り、descendant/child combinator matching は
            // `ancestor_path` — DFS の訪問順そのもの — に依存する。子を
            // 親より先に訪れると `ancestor_path` にまだ親が積まれておらず、
            // 子の combinator matching が誤って不一致になる。旧
            // (raikiri-spike-flln.1 以前) の「訪問順は無関係、
            // 挙動一致のためだけに維持している」という位置づけはここで終わり
            // — 現在は正しさ上の要請。
            let child_depth = ancestor_path.len();
            let start = stack.len();
            stack.extend(dom.child_ids(id).map(|child| (child, child_depth)));
            stack[start..].reverse();
        }
    }
}

/// 1 compound selector 分 — `iter` が次の combinator に達する (または
/// selector 全体の終端に達する) まで — を `elem` 単体に対して判定する。
///
/// [`match_complex_selector_list`] が右端 compound を `elem` 自身に対して
/// 判定する最初の 1 手と、descendant/child combinator 越しの祖先判定
/// ([`match_combinator_chain`] / [`match_from_ancestor`], bd
/// raikiri-spike-flln.2) の両方がこの関数を共有する — bd raikiri-spike-flln.1
/// 時点の (当時の) `match_simple_selectors` 本体をそのまま抽出しただけで、
/// per-component の判定ロジック自体に変更は無い。
///
/// `iter: &mut SelectorIter` を `for component in iter` で回すと、
/// `selectors` crate 自身の contract (`Selector::iter` の doc, verbatim:
/// "Returns an iterator over this selector in matching order
/// (right-to-left). When a combinator is reached, the iterator will return
/// None, and next_sequence() may be called to continue to the next
/// sequence.") により、combinator に達した時点で自動的にループが終わる —
/// `Component::Combinator` 自体がこの for ループの中に component として
/// 出てくることは無い (`SelectorIter::next()` が combinator を internal
/// state に退避して `None` を返す)。呼び出し側は本関数が `false` を返した
/// 場合と「compound は全部一致したが、まだ combinator が続く」場合を
/// 区別する必要があり、後者は呼び出し側が `iter.next_sequence()` で判定する
/// (本関数の戻り値だけでは分からない — 「compound 内で不一致は無かった」を
/// `true` で表すのみ)。
///
/// Returns: この 1 compound 内の全 component が match すれば true。
/// - `Component::LocalName(name)` — `elem.tag_name()` と eq_ignore_ascii_case で判定
/// - `Component::ExplicitUniversalType` — 常に match
/// - `Component::ID` — `elem.id()` と一致比較 (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#id-selectors>、verbatim: "When
///   matching against a document which is in quirks mode, IDs must be
///   matched ASCII case-insensitively; ID selectors are otherwise
///   case-sensitive")。`quirks_mode` 引数が
///   [`StyleQuirksMode::Quirks`] のときのみ `eq_ignore_ascii_case`、それ以外
///   ([`StyleQuirksMode::NoQuirks`] / [`StyleQuirksMode::LimitedQuirks`]) は
///   厳密一致 (bd raikiri-spike-tqwi — "limited-quirks" は DOM Standard上
///   "quirks mode" と別 dfn、fold の対象外)
/// - `Component::Class` — `elem.has_class()` / `elem.has_class_ascii_case_insensitive()`
///   (CSS Selectors L4 <https://www.w3.org/TR/selectors-4/#class-html>、
///   verbatim: "When matching against a document which is in quirks mode,
///   class names must be matched ASCII case-insensitively; class selectors
///   are otherwise case-sensitive")。ID と同じ `quirks_mode` 分岐 (bd
///   raikiri-spike-tqwi)。both variants share the same HTML-spec ASCII
///   whitespace tokenisation — [`StyleElement::has_class`] の doc 参照
/// - `Component::AttributeInNoNamespaceExists` / `Component::AttributeInNoNamespace`
///   — `elem.attr()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#attribute-selectors>)。存在チェック
///   形態 (`[foo]`) の lookup key は element の namespace に応じて
///   `local_name` / `local_name_lower` を選ぶ (詳細は該当 match arm の
///   コメント)。値付き形態の case-sensitivity 解決は
///   [`resolve_case_sensitivity`] 参照
/// - `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
///   (bd raikiri-spike-flln.6) — [`language_range_matches`] /
///   [`resolve_directionality`] 経由、`dom` + `ancestors` (自身の祖先 chain)
///   を使って ancestor-inherited な effective language / directionality を
///   解決する。`PseudoClass::Hover` / `PseudoClass::Active` はこの arm 内で
///   引き続き `false` (bd raikiri-spike-flln.1 の scope 外のまま)。
/// - `Component::Root` (`:root`, bd raikiri-spike-flln.5, CSS Selectors L4
///   §13.1 <https://www.w3.org/TR/selectors-4/#the-root-pseudo>) — matches
///   iff `ancestors.is_empty()`. Both call sites
///   ([`match_complex_selector_list`] for the rightmost compound,
///   [`match_from_element`] for compounds reached by crossing a combinator)
///   pass `ancestors` root-first/immediate-parent-last — [`collect_cascaded`]'s
///   doc establishes that only `StyleNodeKind::Element` nodes are ever
///   pushed onto `ancestor_path`, so an empty `ancestors` slice means "no
///   element ancestor", i.e. this element is the root element of the
///   document tree — exactly the spec's "root of the document" (TR
///   anchor's own prose repeatedly truncated on WebFetch before reaching
///   normative text, same failure mode as [`match_combinator_chain`]'s
///   "Spec provenance note"; what loaded is the summary-table one-liner,
///   "an E element, root of the document", 2026-08-12 direct fetch —
///   sufficient to pin this simple a definition).
/// - `Component::Empty` (`:empty`, bd raikiri-spike-flln.5, CSS Selectors
///   L4 §13.2 <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — see
///   [`matches_empty`] doc for the verbatim spec text and its L4-vs-L3
///   whitespace-handling correction history.
/// - `Component::Nth(data)` (`:first-child`/`:last-child`/`:only-child`/
///   `:nth-child()`/`:nth-last-child()` and their `-of-type` counterparts,
///   bd raikiri-spike-flln.5, CSS Selectors L4 §13.3/§13.4) — see
///   [`matches_nth`] doc for the sibling-position algorithm and its spec
///   citation. Reuses `ancestors.last().copied().unwrap_or_else(||
///   dom.root_id())` for its sibling-list parent — the exact same
///   root-fallback idiom [`match_combinator_chain`]'s `NextSibling`/
///   `LaterSibling` arms already established (bd raikiri-spike-flln.3) for
///   an unrelated reason (sibling lookup key, not a compound-match
///   target); both fall back for the same underlying reason ("the root
///   element's parent-in-tree is the Document node, not an `Element`, but
///   `StyleDom::child_ids` still works against it").
///
/// 他 component (namespace 付き属性 selector = 常に `Component::AttributeOther`、
/// または非小文字 local name **かつ値付き**の属性 selector = 同じく
/// `Component::AttributeOther` — 非小文字でも値なしの存在チェック形態は
/// namespace 無指定なら `AttributeInNoNamespaceExists` のまま、詳細は
/// `ruletree.rs` `is_supported_selector_list` のコメント) は
/// `is_supported_selector_list` が rule tree 構築時点で drop 済のはずだが、
/// safety net として引き続き match fail する。
fn compound_matches<D: StyleDom, E: StyleElement>(
    dom: &D,
    iter: &mut SelectorIter<'_, RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    use selectors::parser::Component;

    for component in iter {
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
            // CSS Selectors L4 id-selectors / class-html (bd
            // raikiri-spike-tqwi, verbatim quoted on the function doc
            // above): ASCII-case-fold only under full quirks mode.
            // `LimitedQuirks` is a *separate* DOM Standard dfn from
            // "quirks mode" (confirmed via direct fetch of
            // <https://dom.spec.whatwg.org/#concept-document-quirks>) and
            // does not get the fold, matching `NoQuirks`.
            Component::ID(id) => match quirks_mode {
                StyleQuirksMode::Quirks => elem
                    .id()
                    .is_some_and(|elem_id| elem_id.eq_ignore_ascii_case(id.0.as_str())),
                StyleQuirksMode::NoQuirks | StyleQuirksMode::LimitedQuirks => {
                    elem.id() == Some(id.0.as_str())
                }
            },
            Component::Class(class) => match quirks_mode {
                StyleQuirksMode::Quirks => elem.has_class_ascii_case_insensitive(class.0.as_str()),
                StyleQuirksMode::NoQuirks | StyleQuirksMode::LimitedQuirks => {
                    elem.has_class(class.0.as_str())
                }
            },
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
            Component::NonTSPseudoClass(pseudo) => match pseudo {
                crate::PseudoClass::Lang(ranges) => {
                    lang_pseudo_matches(ranges, dom, elem, ancestors)
                }
                crate::PseudoClass::Dir(dir) => {
                    resolve_directionality(dom, elem, ancestors) == *dir
                }
                // `:hover` / `:active` — bd raikiri-spike-flln.1 の scope 外
                // のまま。`is_supported_selector_list` が rule tree 構築時点で
                // drop する契約 (`ruletree::tests::pseudo_class_selector_still_dropped`
                // で pin) だが、`match_complex_selector_list_rejects_unsupported_component_via_safety_net`
                // がこの関数を直接呼んで safety net を確認する — 同じ姿勢を
                // 維持。
                crate::PseudoClass::Hover | crate::PseudoClass::Active => false,
            },
            Component::Root => ancestors.is_empty(),
            Component::Empty => matches_empty(dom, elem_id),
            Component::Nth(data) => {
                // Root element (`ancestors.is_empty()`) still has a sibling
                // list — the empty set of *element* siblings under
                // `dom.root_id()` — per CSS Selectors L3 §6.6
                // structural-pseudos preamble's sibling-counting framing
                // (`matches_nth` doc, verbatim); it is not itself excluded
                // just because it has no *element* parent, unlike
                // `Component::Root` above. `dom.root_id()` is the Document
                // node `collect_cascaded` never pushes onto `ancestor_path`
                // (see that function's doc), and its `child_ids` already
                // includes the root element — the natural "sibling-list
                // container" for a root element that in the DOM tree has no
                // element parent at all. Same root-fallback idiom
                // `match_combinator_chain`'s `NextSibling`/`LaterSibling`
                // arms already use (bd raikiri-spike-flln.3).
                let sibling_parent = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
                matches_nth(dom, sibling_parent, elem_id, elem.tag_name(), data)
            }
            _ => {
                // 他 component (AttributeOther) は ruletree build 段で
                // drop 済のはずだが safety net で match fail
                false
            }
        };
        if !component_matches {
            return false;
        }
    }
    true
}

/// CSS Text Module Level 4 "document white space character" (see
/// [`matches_empty`]'s doc for the full verbatim citation and provenance,
/// including the deliberate exclusion of form feed U+000C). For
/// HTML-parsed content this set is `{space, tab, line feed}`; carriage
/// return is included too for defensiveness against a non-HTML-normalized
/// `StyleDom`, even though it cannot occur in a real HTML DOM text node.
fn is_document_white_space(c: char) -> bool {
    matches!(c, '\u{0020}' | '\u{0009}' | '\u{000A}' | '\u{000D}')
}

/// `:empty` (bd raikiri-spike-flln.5, CSS Selectors L4 §13.2
/// <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — whether
/// `elem_id` has no children that count toward emptiness.
///
/// # Spec provenance and correction (reviewer:spec, 2026-08-12)
///
/// L4's own TR anchor repeatedly truncated on WebFetch before reaching
/// normative prose — same failure mode [`match_combinator_chain`]'s "Spec
/// provenance note" documents for combinators. The first pass of this
/// function fell back to Selectors **Level 3** prose ("only... content
/// nodes... whose data has a non-zero length must be considered as
/// affecting emptiness") without realizing L4 had *deliberately changed*
/// this from L3, not merely restated it. Corrected after a direct raw
/// fetch of `raw.githubusercontent.com/w3c/csswg-drafts/main/selectors-4/
/// Overview.bs` (bypassing WebFetch's truncation entirely — `curl` the
/// bikeshed source and grep it directly), `#the-empty-pseudo` section,
/// verbatim: "The :empty pseudo-class represents an element that has no
/// children except, optionally, [=document white space characters=]. ...
/// only element nodes and content nodes (such as [[DOM]] text nodes, and
/// entity references) whose data has a non-zero length must be considered
/// as affecting emptiness; comments, processing instructions, and other
/// nodes must not affect whether an element is considered empty or not."
/// — followed by an explicit changelog note: "In Level 2 and Level 3 of
/// Selectors, :empty did not match elements that contained only white
/// space. This was changed so that... elements which authors perceive of
/// as empty can be selected by this selector, as they expect." The
/// section's own worked examples list `<p> </p>` (whitespace-only) among
/// what `p:empty` matches, and `<div>&nbsp;</div>` among what it does
/// *not* match — pinning both directions.
///
/// "Document white space characters" is itself a CSS Text Module Level 4
/// term (`#the-empty-pseudo`'s own autolink target), verbatim (direct raw
/// fetch of `.../css-text-4/Overview.bs`, `#white-space-rules`): "the
/// [document white space characters]: spaces (U+0020), tabs (U+0009), and
/// segment breaks" — stated a second time nearby, identically: "both
/// include spaces (U+0020), tabs (U+0009), and line feeds (U+000A)". For
/// HTML specifically (same source, `#white-space-rules` preamble), a
/// segment break is exactly line feed (U+000A): "In the case of HTML,
/// newlines are normalized to line feed characters (U+000A)... so... each
/// line feed (U+000A) is treated as a segment break" — and carriage
/// return (U+000D) is separately stated to be "treated identically to
/// spaces (U+0020) in all respects" (same source), even though that same
/// passage confirms CR cannot actually reach a real HTML DOM text node
/// ("carriage returns present in the source code are converted to line
/// feeds at the parsing stage... and therefore do not appear as U+000D...
/// to CSS" — kept here only for defensive completeness against a
/// non-HTML-normalized `StyleDom`, since [`is_document_white_space`] is
/// generic over any `StyleDom` impl, not just `raikiri-html`'s).
///
/// **Deliberately excludes form feed (U+000C)** — unlike Rust's
/// `char::is_ascii_whitespace()` / this crate's own HTML "ASCII
/// whitespace" 5-character set used elsewhere ([`crate::style_dom`]'s
/// `class_token_matches`). Direct search of the css-text-4 raw source
/// (not a WebFetch summary) for "U+000C"/"form feed" returns zero hits
/// anywhere near the "document white space characters" dfn, which is
/// stated explicitly — twice — as exactly {space, tab, segment break/line
/// feed}, no fourth category. This is narrower than an earlier relayed
/// characterization of the set as "U+000A/U+000D/U+000C family" — flagged
/// as a discrepancy for `reviewer:spec` to confirm or correct with a
/// citation, since this function currently follows the directly-verified
/// primary source over the relayed one where they disagree.
///
/// # Node-kind coverage
///
/// [`StyleNodeKind`] has no CDATA/entity-reference variant — HTML parsing
/// produces neither (html5ever folds CDATA-section syntax outside foreign
/// content into a bogus comment per HTML LS tokenization, and HTML has no
/// entity-reference *nodes* the way XML does, only inline character
/// reference expansion during tokenization) — so only `Element` and `Text`
/// need an explicit arm below; `Comment` / `ProcessingInstruction` /
/// `DocumentFragment` (`<template>` contents live in a separate detached
/// tree per that variant's own doc, so they never appear in `child_ids`
/// here regardless) fall through to "does not affect emptiness", matching
/// the spec text's "comments, processing instructions, and other nodes
/// must not affect" clause.
fn matches_empty<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> bool {
    dom.child_ids(elem_id)
        .all(|child_id| match dom.node(child_id) {
            Some(node) => match node.kind() {
                StyleNodeKind::Element => false,
                StyleNodeKind::Text => node
                    .text_content()
                    .unwrap_or("")
                    .chars()
                    .all(is_document_white_space),
                StyleNodeKind::Comment
                | StyleNodeKind::ProcessingInstruction
                | StyleNodeKind::DocumentFragment
                | StyleNodeKind::Document => true,
            },
            // cov:ignore: `child_ids` only ever yields ids that `dom.node`
            // resolves (`StyleDom` trait doc: "child_ids(id) returns an empty
            // iterator for invalid id", implying ids it does yield are valid) —
            // defensive fallback in the same posture as `match_from_element`'s
            // own `dom.node(elem_id)` guard.
            None => true,
        })
}

/// 1-based sibling position of `elem_id` among `parent_id`'s **element**
/// children, both from the start and from the end, plus the total count of
/// such siblings — shared arithmetic behind every `Component::Nth` variant
/// (bd raikiri-spike-flln.5, [`matches_nth`]).
///
/// `of_type == false` (`:nth-child`/`:first-child`/`:last-child`/
/// `:only-child`) counts **all** element siblings regardless of tag; CSS
/// Selectors L3 §6.6 structural-pseudos preamble (verbatim, 2026-08-12
/// direct fetch, <https://www.w3.org/TR/selectors-3/#structural-pseudos> —
/// same feature, L4 does not change this): "Standalone text and other
/// non-element nodes are not counted when calculating the position of an
/// element in its list of siblings; index numbering starts at 1."
///
/// `of_type == true` (`:nth-of-type`/`:first-of-type`/`:last-of-type`/
/// `:only-of-type`) additionally restricts to siblings sharing `elem_tag`
/// — same source, verbatim: "an+b−1 siblings with the same expanded
/// element name". "Expanded element name" is tag name **and** namespace;
/// this crate's `compound_matches` already treats namespace matching as
/// always-true at M1.4 (`Component::DefaultNamespace(_) => true`, "常に
/// match / namespace は m1.4 では常に true 扱い") for the equivalent
/// selector-vs-element case, so restricting this sibling-vs-sibling
/// comparison to `tag_name` equality inherits that existing scope
/// simplification rather than introducing a new one. Plain `==` (not
/// `eq_ignore_ascii_case`, unlike the selector-vs-element `LocalName` arm)
/// — html5ever already normalises HTML tag names to lowercase before they
/// ever reach `StyleElement::tag_name`, and "expanded name" comparison for
/// non-HTML (SVG/MathML) content is case-sensitive per XML tag-name rules,
/// so exact comparison is correct for both.
fn sibling_position<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    of_type: bool,
) -> (i32, i32, i32) {
    let mut total = 0i32;
    let mut index_from_start = 0i32;
    for child_id in dom.child_ids(parent_id) {
        let Some(child_node) = dom.node(child_id) else {
            continue;
        };
        let Some(sibling) = child_node.as_element() else {
            continue;
        };
        if of_type && sibling.tag_name() != elem_tag {
            continue;
        }
        total += 1;
        if child_id == elem_id {
            index_from_start = total;
        }
    }
    let index_from_end = total - index_from_start + 1;
    (index_from_start, index_from_end, total)
}

/// `Component::Nth` — covers `:first-child`/`:last-child`/`:only-child`/
/// `:nth-child()`/`:nth-last-child()` (CSS Selectors L4 §13.3
/// <https://www.w3.org/TR/selectors-4/#the-first-child-pseudo> area) and
/// their `-of-type` counterparts (§13.4
/// <https://www.w3.org/TR/selectors-4/#the-nth-of-type-pseudo> area) — the
/// `selectors` crate itself parses all ten syntaxes into this one
/// `Component` variant, distinguished only by `NthSelectorData::ty`
/// (`selectors` 0.39.0 `parser.rs`'s `NthSelectorData::only`/`first`/`last`
/// constructors and `parse_nth_pseudo_class`, 2026-08-12 direct fetch of
/// the dependency's own public parse dispatch — not a Stylo reference).
///
/// L4's own per-selector TR anchors truncated the same way documented on
/// [`matches_empty`]; fell back to Selectors **Level 3** §6.6
/// <https://www.w3.org/TR/selectors-3/#structural-pseudos> (verbatim,
/// 2026-08-12 direct fetch — again the same feature, unchanged by L4
/// except for the unrelated `An+B of S` extension this task does not
/// implement, see `ruletree.rs`'s `is_supported_selector_list` doc):
/// "The :nth-child(an+b) pseudo-class notation represents an element that
/// has an+b-1 siblings before it in the document tree... The
/// :nth-last-child(an+b) pseudo-class notation represents an element that
/// has an+b-1 siblings after it... :first-child — Same as :nth-child(1)...
/// :last-child — Same as :nth-last-child(1)... :only-child — represents an
/// element that has no siblings... :nth-of-type(an+b) — an element that
/// has an+b-1 siblings with the same expanded element name before it...
/// :only-of-type — an element that has no siblings with the same expanded
/// element name."
///
/// Implementation: convert "an+b−1 siblings before/after" into a 1-based
/// index ([`sibling_position`]) and let `AnPlusB::matches_index` (the
/// `selectors` crate's own An+B arithmetic, already used as-is — no
/// hand-rolled micro-syntax math here) decide. `:only-*` is `total == 1`
/// directly (an element with exactly one matching sibling — itself — has
/// "no siblings" in the relevant filtered sense) rather than routing
/// through `an_plus_b`, matching how `NthSelectorData::only()` fixes
/// `an_plus_b` at a placeholder `AnPlusB(0, 1)` that was never meant to be
/// evaluated for this `ty`.
///
/// The root element (`parent_id` resolved by the caller to `ancestors.last()
/// .copied().unwrap_or_else(|| dom.root_id())` when there is no element
/// ancestor, see the `Component::Nth` arm's own comment in
/// [`compound_matches`]) trivially satisfies `:first-child`/
/// `:last-child`/`:only-child`/`:nth-child(1)` — it has zero element
/// siblings before or after it, which the "an+b-1 siblings before/after
/// it" framing above does not require a *parent element* to state, only a
/// sibling list (possibly of size 1, itself alone).
fn matches_nth<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    data: &NthSelectorData,
) -> bool {
    let (from_start, from_end, total) =
        sibling_position(dom, parent_id, elem_id, elem_tag, data.ty.is_of_type());
    if from_start == 0 {
        // Defensive: `elem_id` was not found among `parent_id`'s (filtered)
        // element children at all — unreachable given `collect_cascaded`'s
        // ancestor-path invariant (every `elem_id`/`parent_id` pair this is
        // ever called with really is a child/parent pair in the walked
        // tree), same posture as `match_from_element`'s own defensive
        // guards. Guards specifically against `AnPlusB(0, 0)`
        // (`:nth-child(0)`, a degenerate but syntactically valid selector)
        // spuriously matching via `matches_index(0) == true` if that
        // invariant were ever violated.
        // cov:ignore: unreachable given the invariant above; would need a
        // `StyleDom` impl that lies about an element's own parent to
        // exercise.
        return false;
    }
    if data.ty.is_only() {
        total == 1
    } else if data.ty.is_from_end() {
        data.an_plus_b.matches_index(from_end)
    } else {
        data.an_plus_b.matches_index(from_start)
    }
}

/// `elem` (と、combinator を跨ぐ場合は `ancestors` で表される祖先 element 列
/// / `elem_id` から辿る兄弟 element 列) と selector list を突き合わせる
/// トップレベル matcher。
///
/// # Combinator 対応 (bd raikiri-spike-flln.2 / flln.3)
///
/// bd raikiri-spike-flln.1 時点は single-element (compound-only) matching
/// のみで、combinator を含む selector は `ruletree.rs`
/// `is_supported_selector_list` の gate で rule tree に乗る前に drop されて
/// いた。bd raikiri-spike-flln.2 で descendant (space, CSS Selectors L4
/// <https://www.w3.org/TR/selectors-4/#descendant-combinators>) と child
/// (`>`, <https://www.w3.org/TR/selectors-4/#child-combinators>) の 2
/// combinator を追加、bd raikiri-spike-flln.3 で adjacent sibling (`+`,
/// <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>) と
/// general sibling (`~`,
/// <https://www.w3.org/TR/selectors-4/#general-sibling-combinators>) を追加
/// (4 combinator 全対応、詳細は [`match_combinator_chain`] doc)。complex
/// selector の一般的な match 条件は CSSWG Editor's Draft
/// <https://drafts.csswg.org/selectors-4/#complex> (verbatim, 2026-08-12
/// 直接 fetch 確認 — provenance の詳細は [`match_combinator_chain`] doc の
/// note 参照) の記述: "A given element ... is said to match a complex
/// selector when it matches the final compound selector ... in the
/// sequence, and every preceding unit of the sequence also matches an
/// element ..., with the correct relationship between consecutive units as
/// expressed by the combinators separating them" — 本関数はこれを右 (elem
/// 自身) から左 (祖先/兄弟) への
/// `Selector::iter`/`SelectorIter::next_sequence` の反復として実装する:
///
/// 1. 一番右の compound を `elem` 自身に対して [`compound_matches`] で判定。
/// 2. 不一致ならこの selector は不一致、次の selector へ。
/// 3. 一致すれば `iter.next_sequence()` で次の combinator を見る:
///    - `None` (もう combinator が無い) → selector 全体が一致。
///    - `Some(combinator)` → [`match_combinator_chain`] に委譲、combinator
///      の意味 (child = 直近の親のみ、descendant = いずれかの祖先、
///      next-sibling = 直前の兄弟のみ、later-sibling = それ以前のいずれかの
///      兄弟) に沿って次の compound を判定する。
///
/// `ancestors` は root 側が先頭、直近の親が末尾の順 (`ancestors.last()` ==
/// `elem` の親) — [`collect_cascaded`] の DFS 訪問順から構築される
/// (同関数の doc 参照)。`elem_id` は `elem` 自身の id — bd raikiri-spike-flln.3
/// で sibling combinator が「`elem` の親の子リストの中で `elem` より前にいる
/// のは誰か」を [`StyleDom::child_ids`] から直接求める際の探索終端として
/// 導入され ([`match_combinator_chain`] の `NextSibling`/`LaterSibling` arm
/// 参照)、bd raikiri-spike-flln.5 で [`compound_matches`] 自身にも渡すよう
/// 拡張 — `:root`/`:empty`/`:nth-child()` 等の構造的 pseudo-class が
/// `dom`/`elem_id`/`ancestors.last()` (= `elem` の親) を必要とするため、
/// `elem` の借用値だけでは表現できない情報として渡す。
///
/// Returns: matching した selector の最大 specificity。1 つも match しなければ None。
/// `specificity_of` は selector 全体 (combinator を跨いだ複合 selector) に
/// 対する値 — combinator 追加後もこの呼び出しに変更は無い (`selectors`
/// crate 自身が selector 全体から算出する)。
fn match_complex_selector_list<D: StyleDom, E: StyleElement>(
    list: &SelectorList<RaikiriSelectorImpl>,
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> Option<Specificity> {
    let mut best: Option<Specificity> = None;
    for selector in list.slice() {
        let mut iter = selector.iter();
        let whole_matches = compound_matches(dom, &mut iter, elem, elem_id, ancestors, quirks_mode)
            && match iter.next_sequence() {
                None => true,
                Some(combinator) => {
                    match_combinator_chain(dom, combinator, elem_id, ancestors, iter, quirks_mode)
                }
            };
        if whole_matches {
            let spec = specificity_of(selector);
            best = Some(match best {
                Some(prev) => prev.max(spec),
                None => spec,
            });
        }
    }
    best
}

/// [`match_complex_selector_list`] が右端 compound を `elem` に対して
/// マッチさせたあと、残りの combinator + compound 列を `ancestors`
/// (祖先チェーン) / `current_id` から辿る兄弟列のどちらかを遡って判定する。
///
/// - [`Combinator::Child`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim: "A
///   child combinator describes a childhood relationship between two
///   elements") — 候補は `ancestors` の末尾 (直近の親) **1 つだけ**。それが
///   次の compound に一致し、かつ (さらに左に combinator が続くなら) その
///   親のそのまた祖先から続きが一致すれば全体一致。バックトラックは無い —
///   `>` は「直近の親」を一意に指すため。
/// - [`Combinator::Descendant`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#descendant-combinators>, verbatim:
///   "A selector of the form A B represents an element B that is an
///   arbitrary descendant of some ancestor element A") — `ancestors` を
///   直近の親から根に向かって 1 つずつ試し、次の compound が一致した候補を
///   見つけたら、その候補を起点にさらに左の残りを再帰的に判定する。1 候補で
///   残りの判定まで失敗した場合、次の (さらに外側の) 祖先で再試行する —
///   `iter.clone()` (`selectors::parser::SelectorIter` は `Clone`) で候補
///   ごとに独立した iterator コピーを使う。CSSWG Editor's Draft
///   <https://drafts.csswg.org/selectors-4/#complex> の complex selector
///   定義の「(match の条件は) 各 unit が対応する combinator の関係を
///   満たしながら何らかの element に一致すること」という再帰的な定義を
///   そのまま素直に実装したもの。
///
///   **この再試行は load-bearing — 省略すると壊れる** (2026-08-12 訂正:
///   以前ここには「祖先チェーンは分岐の無い単一の直線なので retry は
///   冗長」という誤った一般化があった。reviewer:spec / reviewer:quality /
///   reviewer:debt 3 lens が独立に同型の反例を構築して指摘 — 以下は
///   その反例)。誤りだった論法は「直近候補を選んだ場合の残り
///   `ancestors` は、より遠い候補を選んだ場合の残り `ancestors` を
///   必ず包含する superset になる」という主張だったが、これは**残りが
///   すべて [`Combinator::Descendant`] のとき**にしか成立しない —
///   その場合は次の判定が「残り `ancestors` の**どこかに** compound が
///   一致するか」という集合に対する自由な存在探索で、探索対象が広い
///   ほど (superset ほど) 弱くならないため。しかし残りに
///   [`Combinator::Child`] が 1 つでも混ざると、その段の判定は
///   `ancestors.split_last()` が指す**特定 1 要素**の compound 一致
///   可否であり、候補ごとに「集合の一部を切り詰めたもの」ではなく
///   「そもそも別の要素」を見ることになる — supersetによる包含関係が
///   意味を持たない。
///
///   反例 (`.x > .y .target`、`x`/`y`/`target` は class):
///   `G(.x) → F(.y) → M(no class) → C(.y) → elem(.target)` という祖先
///   チェーンで `elem` を判定する。`elem` の直近の `.y` 候補は `C`
///   だが、`C` の直近の親は `M` で `.x` を持たない — `Child` の判定対象
///   `M` に固定されるため、`C` 候補はここで確定的に失敗する。ここで
///   打ち切ると selector 全体が不一致になってしまうが、正しい答えは
///   一致: より遠い候補 `F` (`.y` を持つ) の直近の親は `G` で `.x` を
///   持つ。`F` を試すこの再試行が無ければ、この (spec 上正当な)
///   selector が静かに一致しなくなる — pin 用の regression test
///   `tests::descendant_retry_past_a_failed_child_combinator_candidate_is_required`
///   (この module 内 `#[cfg(test)] mod tests`) がこの具体形をそのまま
///   実行する。
///
///   sibling combinator (`+`/`~`) のような非祖先チェーン型 combinator が
///   同じ complex selector 内に混在するとさらに事情が変わりうる — ただし
///   bd raikiri-spike-flln.3 で判明した通り「事情が変わる」というのは
///   「不正確になる」ではなく「別の軸で load-bearing になる」だった:
///   sibling ジャンプは `ancestors` を不変のまま引き継ぐため、そこから
///   さらに左へ [`Combinator::Descendant`] が続く場合もこの retry は
///   同じ理由でそのまま load-bearing (下記 [`Combinator::NextSibling`] /
///   [`Combinator::LaterSibling`] の説明、および `ruletree.rs`
///   `is_supported_selector_list` doc の "4 combinator 間の混在" note
///   参照)。探索順序 (直近から遠方へ)
///   自体は正しさに影響しない — いずれの順で候補を試しても最終的な
///   一致/不一致の結果 (「一致する候補が存在するか」という真偽値) は
///   変わらない。
/// - [`Combinator::NextSibling`] (bd raikiri-spike-flln.3, CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>
///   §14.3, verbatim: "The elements represented by the two compound
///   selectors share the same parent in the document tree and the element
///   represented by the first compound selector immediately precedes the
///   element represented by the second one. Non-element nodes (e.g. text
///   between elements) are ignored when considering the adjacency of
///   elements.") — 候補は `current_id` の親 (`ancestors.last()`、無ければ
///   [`StyleDom::root_id`]、下記 note 参照) の子リストの中で `current_id`
///   の**直前**の element 1 つだけ ([`immediate_preceding_sibling`])。
///   バックトラックは無い — `+` は「直前の兄弟」を一意に指すため
///   ([`Combinator::Child`] と同じ形)。
/// - [`Combinator::LaterSibling`] (bd raikiri-spike-flln.3, CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#general-sibling-combinators> §14.4,
///   verbatim: "The elements represented by the two compound selectors
///   share the same parent in the document tree and the element
///   represented by the first compound selector precedes (not necessarily
///   immediately) the element represented by the second one.") —
///   `current_id` の親の子リストを先頭から順に試し、`current_id` に達したら
///   打ち切る。一致する候補が見つかり次第、その候補を起点にさらに左の残りを
///   再帰的に判定する ([`Combinator::Descendant`] と同じ「単一の直線を
///   バックトラックする」形 — 兄弟リストも分岐が無いため、探索順序は正しさに
///   影響しない。ここでは `child_ids` が返す自然な順序 (先頭 = 最も遠い兄弟)
///   のまま辿る)。
///
/// # 親の解決: `ancestors.last()` の空スライス fallback (bd raikiri-spike-flln.3)
///
/// `ancestor_path` は **Element kind の node のみ**を積む
/// ([`collect_cascaded`] doc 参照) ので、`current_id` の親が
/// [`StyleNodeKind::Document`] root 自身であるとき (= document 直下の
/// element、`<html>` 等) `ancestors` は空になる — `Child`/`Descendant` は
/// この場合を「親が compound に一致し得ない」= 不一致として正しく扱う
/// (`ancestors.split_last() => None`) が、sibling combinator は**親自身を
/// compound と照合するわけではなく**、[`StyleDom::child_ids`] の lookup key
/// として親の id が要るだけ — root であっても兄弟は実在しうる (`<h2>` と
/// `<p>` が両方 document 直下の兄弟、という acceptance のケースそのもの)。
/// そのため `NextSibling`/`LaterSibling` の 2 arm だけ `ancestors.last()` が
/// `None` のとき [`StyleDom::root_id`] にフォールバックする — `Child`/
/// `Descendant` 側はこのフォールバックを持たない (持ってはならない — root は
/// 決して compound に一致しない)。
///
/// 他 combinator ([`Combinator::PseudoElement`] / [`Combinator::SlotAssignment`]
/// / [`Combinator::Part`]) はこの task の scope 外 — `ruletree.rs`
/// `is_supported_selector_list` が rule tree 構築時点で drop するのに加え、
/// これら 3 つは pseudo-element 専用の combinator で、本 crate の
/// `parse_selector_list` (`RaikiriSelectorImpl`) がそもそも pseudo-element
/// 構文自体を `Custom(UnsupportedPseudoClassOrElement(..))` として parse
/// error にする (bd raikiri-spike-flln.3 で `a::before` を直接 parse させて
/// 実地確認、2026-08-12) ため、この crate 内で生成された `SelectorList` から
/// 到達することは無い。[`compound_matches`] の `_ => false` safety net と
/// 同じ姿勢で、ここでも到達したら match fail 扱いにする。
///
/// # Spec provenance note (bd raikiri-spike-flln.2 / flln.3, 2026-08-12)
///
/// この doc および [`match_complex_selector_list`] / [`collect_cascaded`]
/// が引用する verbatim 文言はすべて、`https://www.w3.org/TR/selectors-4/`
/// への直接 WebFetch がページ全体の大きさのため section 14 (Combinators) は
/// おろか `#complex` (§4) にすら到達する前に繰り返し切り詰められたことを
/// 受け、代わりに同一文書の正典 source である CSSWG bikeshed 原稿
/// (`raw.githubusercontent.com/w3c/csswg-drafts/main/selectors-4/Overview.bs`,
/// 2026-08-12 直接 fetch) から確認したもの — TR ページの当該 anchor への
/// 直接到達はできていない。descendant/child/next-sibling/general-sibling
/// combinator の文言 (定義文中心の安定した記述、4 つとも同じ `<h3 id=…>`
/// 形式の見出し直下) はこの ED 原稿の内容が publish 済み TR とも一致して
/// いると見込んで TR anchor (`#descendant-combinators` /
/// `#child-combinators` / `#adjacent-sibling-combinators` /
/// `#general-sibling-combinators`) に紐付けたままにしているが、`#complex`
/// (complex selector 全体の match 条件) は ED 側の周辺記述に
/// pseudo-compound selector 関連の、TR 発行後に追加された可能性のある文言が
/// 混在しており、そちらは "TR と一致しているはず" という前提を置かず ED URL
/// (<https://drafts.csswg.org/selectors-4/#complex>) 自体に紐付けている
/// ([`match_complex_selector_list`] の引用も同様)。§14.3/§14.4 の節番号は
/// 同じ ED 原稿内の `<h2 id="combinators">` 配下の `<h3>` 出現順
/// (descendant, child, adjacent-sibling, general-sibling — 2026-08-12
/// 直接確認) から数えたもの。
///
/// # Implementation: explicit `Vec` stack, not native recursion (bd raikiri-spike-8r16)
///
/// Prior to bd raikiri-spike-8r16 this function and [`match_from_element`]
/// mutually recursed on the native Rust call stack — one stack frame pair
/// per combinator actually walked while matching successively along the
/// ancestor/sibling chain, with no selector-length/complexity cap anywhere
/// in the parse/build path. That is the same class of problem
/// `collect_cascaded` had pre-job-199 (tree-depth-correlated native
/// recursion on untrusted-depth input) — confirmed empirically here too:
/// regression test `deep_child_combinator_chain_small_stack_no_overflow`
/// (this module's `tests`) reliably aborted the process with a native stack
/// overflow (128 KiB stack, 500-deep uniformly-matching `>` chain) against
/// the prior recursive implementation.
///
/// The fix below uses an explicit `Vec`-based stack, the same *technique*
/// `collect_cascaded` uses for its own job-199 fix — but not the same
/// *shape*: `collect_cascaded` is a plain DFS with no backtracking (visit
/// every node once), whereas [`Combinator::Descendant`] /
/// [`Combinator::LaterSibling`] must try multiple candidates in order and
/// fall back to the next one when a deeper match fails entirely (see this
/// doc's "load-bearing" retry note above). So each stack entry here is a
/// *choice point* (a [`PendingCandidates`] cursor over not-yet-tried
/// candidates for one level of the chain, plus the ancestors to hand a
/// matched candidate and the [`SelectorIter`] to resume from) rather than a
/// bare node id — pushed when a candidate's compound matches and a further
/// combinator remains (going one level deeper/further left), popped when a
/// level's candidates are exhausted (backtracking to the next candidate of
/// the parent choice point). A full match short-circuits immediately
/// (`return true`) without draining the stack; only exhausting the
/// outermost choice point's candidates yields an overall `false`.
///
/// The word "再帰的に" ("recursively") in the per-combinator prose above
/// describes the *logical* structure of the search — CSS complex selectors
/// are themselves defined recursively (CSSWG ED `#complex`, cited above) —
/// not this function's implementation technique; that logical recursion is
/// realized here as the explicit stack's push/pop, never the native call
/// stack.
fn match_combinator_chain<D: StyleDom>(
    dom: &D,
    combinator: Combinator,
    current_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    iter: SelectorIter<'_, RaikiriSelectorImpl>,
    quirks_mode: StyleQuirksMode,
) -> bool {
    struct Frame<'a, 's, D: StyleDom> {
        candidates: PendingCandidates<'a, D>,
        /// Ancestors [`Combinator::NextSibling`]/[`Combinator::LaterSibling`]
        /// candidates are evaluated against — unchanged across every
        /// candidate in this frame (siblings share a parent). Unused by
        /// [`Combinator::Child`]/[`Combinator::Descendant`], whose
        /// candidates carry their own (shrinking) ancestors directly out of
        /// `PendingCandidates::next` instead.
        ancestors_unchanged: &'a [StyleNodeId],
        /// Positioned at the compound this frame's combinator introduced;
        /// cloned fresh for every candidate attempt (`Descendant`/
        /// `LaterSibling` mirror the pre-fix code's own per-candidate
        /// `iter.clone()`; `Child`/`NextSibling` are newly cloned here too,
        /// for uniform frame handling across all four combinators — each
        /// has exactly one candidate, so the pre-fix code moved `iter`
        /// instead of cloning it. Negligible cost, no heap allocation; see
        /// perf lens's bd raikiri-spike-bj4p follow-up for the broader
        /// allocation picture).
        iter: SelectorIter<'s, RaikiriSelectorImpl>,
    }

    let mut stack = vec![Frame {
        candidates: pending_candidates_for(dom, combinator, current_id, ancestors),
        ancestors_unchanged: ancestors,
        iter,
    }];
    loop {
        let Some(frame) = stack.last_mut() else {
            // Outermost choice point exhausted with no full match found.
            return false;
        };
        let Some((candidate_id, candidate_ancestors)) =
            frame.candidates.next(dom, frame.ancestors_unchanged)
        else {
            // This level's candidates are exhausted — backtrack to the
            // parent choice point's next candidate.
            stack.pop();
            continue;
        };
        let candidate_iter = frame.iter.clone();
        let Some(mut matched_iter) = match_from_element(
            dom,
            candidate_id,
            candidate_ancestors,
            candidate_iter,
            quirks_mode,
        ) else {
            // Candidate's compound didn't match — try this frame's next
            // candidate (loop back without push/pop).
            continue;
        };
        match matched_iter.next_sequence() {
            // No further combinator to the left: the whole complex
            // selector matched.
            None => return true,
            Some(next_combinator) => {
                // Compound matched and more remains further left — descend
                // one level (push a new choice point) rather than recurse.
                stack.push(Frame {
                    candidates: pending_candidates_for(
                        dom,
                        next_combinator,
                        candidate_id,
                        candidate_ancestors,
                    ),
                    ancestors_unchanged: candidate_ancestors,
                    iter: matched_iter,
                });
            }
        }
    }
}

/// Not-yet-tried candidates for one [`match_combinator_chain`] choice point
/// — the iterative counterpart of that function's four `match combinator`
/// arms' candidate-generation logic (bd raikiri-spike-8r16). Each variant
/// corresponds 1:1 to a [`Combinator`] arm; see [`pending_candidates_for`]
/// for the construction side and [`match_combinator_chain`]'s "Implementation"
/// doc for why this needs to be a resumable cursor rather than a one-shot
/// iterator (backtracking may resume a frame after a deeper level failed).
enum PendingCandidates<'a, D: StyleDom + 'a> {
    /// [`Combinator::Child`]: exactly one candidate
    /// (`ancestors.split_last()`'s parent, paired with the remaining
    /// ancestors above it) — `None` once taken, or if there was no parent
    /// to begin with. No backtracking past this single candidate, matching
    /// the pre-fix code's non-looping `match ancestors.split_last() { .. }`.
    Child(Option<(StyleNodeId, &'a [StyleNodeId])>),
    /// [`Combinator::Descendant`]: remaining ancestors to try, closest-first
    /// — mirrors the pre-fix code's `while let Some((&id, further)) =
    /// remaining.split_last()` loop. Each candidate is handed the *further*
    /// ancestors (everything above it) both as its own matching context and
    /// as the next resume point.
    Descendant(&'a [StyleNodeId]),
    /// [`Combinator::NextSibling`]: exactly one candidate (the immediate
    /// preceding sibling, if any), evaluated against the frame's unchanged
    /// `ancestors_unchanged`.
    NextSibling(Option<StyleNodeId>),
    /// [`Combinator::LaterSibling`]: children of the shared parent up to
    /// (excluding) `stop_at`, in document order — mirrors the pre-fix
    /// code's single-pass `for candidate_id in dom.child_ids(parent_id) {
    /// if == current_id { break } .. }` loop. Holds a *live* `D::ChildIter`
    /// (not a re-derived one) so resuming this frame after a deeper level
    /// fails continues exactly where the previous attempt left off — same
    /// single left-to-right pass as the original, no re-scan, no throwaway
    /// `Vec` (same convention [`immediate_preceding_sibling`]'s doc
    /// establishes).
    LaterSibling {
        child_iter: D::ChildIter<'a>,
        stop_at: StyleNodeId,
    },
}

impl<'a, D: StyleDom + 'a> PendingCandidates<'a, D> {
    /// Advances to (and returns) the next untried candidate, paired with
    /// the ancestors it should be evaluated against, or `None` once this
    /// choice point is exhausted. `ancestors_unchanged` is the frame's own
    /// field (not stored on `Self` — only the [`Combinator::NextSibling`]/
    /// [`Combinator::LaterSibling`] variants need it, and every candidate
    /// within a frame needs the *same* value, so the caller threads it
    /// through rather than duplicating it per variant).
    fn next(
        &mut self,
        dom: &D,
        ancestors_unchanged: &'a [StyleNodeId],
    ) -> Option<(StyleNodeId, &'a [StyleNodeId])> {
        match self {
            Self::Child(slot) => slot.take(),
            Self::Descendant(remaining) => {
                let (&candidate_id, further) = remaining.split_last()?;
                *remaining = further;
                Some((candidate_id, further))
            }
            Self::NextSibling(slot) => slot.take().map(|id| (id, ancestors_unchanged)),
            Self::LaterSibling {
                child_iter,
                stop_at,
            } => {
                for candidate_id in child_iter.by_ref() {
                    if candidate_id == *stop_at {
                        // Reached `current_id` itself: no more candidates
                        // are ever valid past this point (mirrors the
                        // pre-fix code's `break`). Not resetting the
                        // iterator is fine — a choice point that returned
                        // `None` once is never queried again by
                        // `match_combinator_chain`'s driving loop.
                        return None;
                    }
                    if is_in_document_element(dom, candidate_id) {
                        return Some((candidate_id, ancestors_unchanged));
                    }
                }
                // cov:ignore: same invariant as `immediate_preceding_sibling`'s
                // own trailing `None` — `stop_at` (`current_id`) is always one
                // of `parent_id`'s own children (it is `pending_candidates_for`'s
                // own `current_id` argument, and `parent_id` is derived from
                // it via `ancestors.last()`), so the loop above always returns
                // via the `*stop_at` branch before `child_iter` is exhausted.
                // Would need a `StyleDom` impl whose `child_ids(parent_id)`
                // omits an id it itself supplied as `current_id` to exercise.
                None
            }
        }
    }
}

/// Builds the [`PendingCandidates`] cursor for one combinator, mirroring
/// [`match_combinator_chain`]'s pre-fix per-combinator candidate-generation
/// logic exactly (bd raikiri-spike-8r16) — this function does no matching
/// itself, only candidate enumeration setup. The `_ => ..` safety-net arm
/// (unsupported combinators, see this module's "他 combinator" doc note)
/// yields an already-exhausted `Child(None)` cursor, the same "no candidate
/// ever succeeds" outcome the pre-fix `_ => false` arm produced.
fn pending_candidates_for<'a, D: StyleDom + 'a>(
    dom: &'a D,
    combinator: Combinator,
    current_id: StyleNodeId,
    ancestors: &'a [StyleNodeId],
) -> PendingCandidates<'a, D> {
    match combinator {
        Combinator::Child => PendingCandidates::Child(
            ancestors
                .split_last()
                .map(|(&parent_id, rest)| (parent_id, rest)),
        ),
        Combinator::Descendant => PendingCandidates::Descendant(ancestors),
        Combinator::NextSibling => {
            let parent_id = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
            PendingCandidates::NextSibling(immediate_preceding_sibling(dom, parent_id, current_id))
        }
        Combinator::LaterSibling => {
            let parent_id = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
            PendingCandidates::LaterSibling {
                child_iter: dom.child_ids(parent_id),
                stop_at: current_id,
            }
        }
        // cov:ignore: `Combinator::PseudoElement`/`SlotAssignment`/`Part` are
        // the only other `Combinator` variants and this crate's own
        // `parse_selector_list` never produces a `SelectorList` containing
        // them (see this module's "他 combinator" doc note, above
        // `match_combinator_chain`, for the full argument — same posture as
        // `compound_matches`'s own `_ => false` safety net for unsupported
        // `Component` variants). Would need a `SelectorList` built by some
        // means other than this crate's own parser to exercise.
        _ => PendingCandidates::Child(None),
    }
}

/// `parent_id`'s direct children (document order) が `Element` kind かつ
/// [`StyleNode::is_in_document`] であるかを判定する共有述語。
/// [`immediate_preceding_sibling`] と [`match_combinator_chain`] の
/// `LaterSibling` arm の両方から使う — [`collect_cascaded`] が
/// `ancestor_path` に積む前に行う `!node.is_in_document() => continue` gate
/// (同関数の doc 参照) と同じ基準を、sibling 側の候補選定でも揃えるための
/// 抽出 (bd raikiri-spike-flln.3) — 揃えないと `<template>` 子孫のような
/// inert element が sibling combinator の候補として拾われてしまう。
fn is_in_document_element<D: StyleDom>(dom: &D, id: StyleNodeId) -> bool {
    dom.node(id)
        .is_some_and(|node| node.is_in_document() && node.kind() == StyleNodeKind::Element)
}

/// `parent_id` の直接の子のうち、`current_id` の**直前**にいる element の id
/// ([`Combinator::NextSibling`] 用)。[`StyleDom::child_ids`] を先頭から 1
/// パス走査し、`current_id` に達した時点でそれまでに見た最後の element
/// candidate を返す — 割り当ては行わない (`Vec` 不使用、bd
/// raikiri-spike-75ch の「使い捨て `Vec` を経由しない」方針を踏襲)。
///
/// Non-element node (text 等) は候補から除外 — CSS Selectors L4
/// next-sibling combinator 自身の verbatim: "Non-element nodes (e.g. text
/// between elements) are ignored when considering the adjacency of
/// elements" (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>)。
fn immediate_preceding_sibling<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    current_id: StyleNodeId,
) -> Option<StyleNodeId> {
    let mut last_element = None;
    for candidate_id in dom.child_ids(parent_id) {
        if candidate_id == current_id {
            return last_element;
        }
        if is_in_document_element(dom, candidate_id) {
            last_element = Some(candidate_id);
        }
    }
    // cov:ignore: `current_id` is always one of `parent_id`'s own children
    // when this helper is called — `parent_id` is derived from `current_id`
    // itself (either `current_id`'s own parent via `ancestors.last()`, or —
    // when `current_id` was reached through a prior sibling/ancestor jump —
    // the parent shared with the node that produced it, see
    // `match_combinator_chain`'s callers). Would need a `StyleDom` impl
    // whose `child_ids(parent_id)` omits an id it itself supplied as
    // `ancestors.last()` / a sibling candidate to exercise this branch.
    None
}

/// `elem_id` の element を解決し、[`compound_matches`] で `iter` が指す
/// compound をそれに対して判定する。祖先候補 (`Child`/`Descendant`) と
/// 兄弟候補 (`NextSibling`/`LaterSibling`) の両方がこの 1 つの関数を共有する
/// — 「id を解決して compound を照合する」というロジック自体は候補がどちらの
/// combinator 由来かに依存しない (bd raikiri-spike-flln.3: `ancestors` は
/// 兄弟ジャンプでは不変のまま引き継がれる — 兄弟は親を共有するため — ことが
/// この共有を成立させる。祖先ジャンプでは従来通り `split_last`/バックトラック
/// で truncate 済みの残り `ancestors` を渡す)。旧名 `match_from_ancestor`
/// (bd raikiri-spike-flln.2) — 兄弟候補にも使われるようになったため
/// bd raikiri-spike-flln.3 で `match_from_element` に rename。
///
/// bd raikiri-spike-8r16 より前は、compound が一致した後さらに左の
/// combinator へ**自分で再帰**していた ([`match_combinator_chain`] との
/// 相互再帰、native stack を消費する側)。現在は compound 一致後の `iter`
/// (次の compound の手前まで進んだ状態) を `Some` で返すだけに変わり、
/// 「さらに左の combinator へ進むかどうか」の判断とその実行は
/// [`match_combinator_chain`] の explicit `Vec` stack 駆動ループ側の責務に
/// 一本化されている (同関数の "Implementation" doc 参照) — 呼び出し側が
/// 自分の判断で `matched_iter.next_sequence()` を呼び、`stack.push` するか
/// `return true` するかを選ぶ。
///
/// `elem_id` を [`StyleElement`] の借用値ではなく [`StyleNodeId`] で受け取る
/// 設計: `StyleElement` は [`StyleDom::NodeRef`]/[`StyleNode::Element`] と
/// いう GAT 経由の型で、呼び出しをまたいで別の借用ライフタイムの値を
/// 持ち回るにはシグネチャが煩雑になる — id は `Copy` なのでこの受け渡しには
/// 明らかに軽量。[`collect_cascaded`] 側で既に解決済みの `elem` を再利用
/// しない分、候補 1 段ごとに `dom.node()`/`as_element()` を 1 回余分に
/// 呼ぶが、raikiri-style crate-internal な `#[cfg(test)]` 限定 mock
/// (`TestDoc`) / raikiri-dom の実装いずれも arena index 参照相当の安価な
/// lookup (`TestDoc` の宿る module は `#[cfg(test)]` gated のため、ここは
/// あえて intra-doc link 化しない — non-test の `cargo doc` からは解決
/// できない target になる)。
///
/// Returns: compound が一致すれば、その後の compound を指す `iter` を
/// `Some` で返す (呼び出し側がさらに左へ進めるかどうかを判断する)。
/// 一致しなければ `None`。
fn match_from_element<'s, D: StyleDom>(
    dom: &D,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    mut iter: SelectorIter<'s, RaikiriSelectorImpl>,
    quirks_mode: StyleQuirksMode,
) -> Option<SelectorIter<'s, RaikiriSelectorImpl>> {
    // Both guards below are defensive and not reachable via the real
    // `collect_cascaded` → `match_complex_selector_list` call path: every
    // `elem_id` this function is ever invoked with comes from one of two
    // sources, both already filtered to Element-kind + in-document ids —
    // `ancestors` (built by `collect_cascaded`'s `ancestor_path`, which only
    // ever pushes an id inside its `node.kind() == StyleNodeKind::Element`
    // branch after the `!node.is_in_document() => continue` gate — see that
    // function's doc), or a sibling candidate already passed through
    // `is_in_document_element` (`immediate_preceding_sibling` /
    // `PendingCandidates::LaterSibling`). So `dom.node(elem_id)` is always
    // `Some`, and its `as_element()` is always `Some` too. Kept as an
    // explicit safety net rather than `.unwrap()`/`unreachable!()` — same
    // defensive posture as `compound_matches`'s own `_ => false` arm for
    // unsupported `Component` variants — because `StyleDom`/`StyleElement`
    // are generic traits not owned by this crate; a future non-test
    // implementation could theoretically violate the invariant.
    // cov:ignore: unreachable given the construction invariants above;
    // would need a `StyleDom` impl that returns `None`/non-Element for an id
    // it itself supplied as an ancestor or a filtered sibling candidate to
    // exercise.
    let node = dom.node(elem_id)?;
    // cov:ignore: see the guard immediately above — same invariant.
    let elem = node.as_element()?;
    // `ancestors` here is *this* element's own remaining ancestor chain
    // (root-most first) — `pending_candidates_for`'s `Child`/`Descendant`
    // arms hand out `rest`/`further` (everything left after popping
    // `elem_id` itself off the end), so `ancestors.last()` is `elem_id`'s
    // parent, exactly mirroring `match_complex_selector_list`'s own use of
    // `ancestors` for the rightmost compound (bd raikiri-spike-flln.5) —
    // needed so a structural pseudo-class in a non-rightmost compound
    // (e.g. `body > div:only-child p`) resolves against the right parent,
    // not `elem`'s (the search's original caller's) parent. Sibling jumps
    // (bd raikiri-spike-flln.3) pass `ancestors` through unchanged (siblings
    // share a parent), so this holds for those candidates too.
    if !compound_matches(dom, &mut iter, &elem, elem_id, ancestors, quirks_mode) {
        return None;
    }
    Some(iter)
}

// ---------------------------------------------------------------------------
// `:lang()` / `:dir()` (bd raikiri-spike-flln.6).
//
// Both pseudo-classes resolve a property of the element that is NOT a plain
// own-attribute lookup — CSS Selectors L4 explicitly distinguishes them from
// the attribute-selector equivalent (`[lang|=C]` / `[dir=C]`) precisely
// because they consult "the UA's knowledge of the document's semantics"
// (`:dir()`'s own wording, quoted on `resolve_directionality`'s doc) —
// concretely, ancestor inheritance. Both therefore reuse the same
// `ancestors: &[StyleNodeId]` (root-first, immediate-parent-last) that
// `compound_matches` already threads through for descendant/child combinator
// matching (bd raikiri-spike-flln.2) — self is checked first, then
// `ancestors` is walked from `.last()` (immediate parent) toward `.first()`
// (document root).
// ---------------------------------------------------------------------------

/// `PseudoClass::Lang` arm of [`compound_matches`] — CSS Selectors L4 §7.2
/// <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>: "represents an
/// element whose content language is one of the languages listed in its
/// argument" (bikeshed source verbatim, see [`language_range_matches`] doc
/// for the fetch note). `ranges` is empty-or-more per [`PseudoClass::Lang`]
/// grammar (`parse_comma_separated` never actually returns an empty `Vec`
/// for a non-empty `:lang(...)` argument list, but this function does not
/// special-case emptiness — `ranges.iter().any(..)` is vacuously `false` on
/// an empty slice, the same "never matches" outcome an empty argument list
/// should have, so no explicit guard is needed even if that upstream
/// guarantee ever changes). An element with no resolvable content language
/// never matches, regardless of `ranges` (this also implements the spec's
/// wildcard-range special case for free, see [`effective_language`] doc).
fn lang_pseudo_matches<D: StyleDom, E: StyleElement>(
    ranges: &[String],
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> bool {
    match effective_language(dom, elem, ancestors) {
        Some(lang) => ranges
            .iter()
            .any(|range| language_range_matches(range, &lang)),
        None => false,
    }
}

/// Resolves an element's **content language** per HTML Living Standard
/// §3.2.6.2 "The `lang` and `xml:lang` attributes"
/// (<https://html.spec.whatwg.org/multipage/dom.html#the-lang-and-xml:lang-attributes>,
/// 2026-08-12 direct fetch), simplified to the subset of "determine the
/// language of a node" this crate can express:
///
/// > To determine the language of a node, user agents must use the first
/// > appropriate step in the following list: \[...\] If the node is an HTML
/// > element or an element in the SVG namespace, and it has a lang in no
/// > namespace attribute set — Use the value of that attribute. \[...\] If
/// > the node's parent element is not null — Use the language of that
/// > parent element. Otherwise \[...\] the language of the node is unknown,
/// > and the corresponding language tag is the empty string.
///
/// i.e. own `lang` attribute wins; absent, walk up to the nearest ancestor
/// that has one; absent everywhere (no pragma-set default / protocol-level
/// language either, both out of scope — this crate has no HTTP layer and
/// does not parse `<meta http-equiv=content-language>`), the language is
/// unknown.
///
/// Like [`own_explicit_direction`]'s `dir` reads, the **own**-attribute
/// step gates on `elem.namespace_uri()` — but a 2-element allowlist (HTML
/// *or* SVG) rather than `dir`'s HTML-only 1-element one, per the quoted
/// step's explicit "an HTML element or an element in the SVG namespace"
/// wording (reviewer:spec finding, 2026-08-12: an earlier version of this
/// function read `lang` unconditionally, which is wrong for any other
/// foreign-namespace element — MathML concretely: `<math lang="ja">` nested
/// under `<html lang="en">` must resolve to `"en"`, not `"ja"`, since MathML
/// is neither HTML nor SVG. [`own_html_or_svg_lang_attribute`] is the gate;
/// see its doc for the allowlist). This only restricts *whose own*
/// attribute counts — the ancestor walk below still applies the same gate
/// per ancestor (a MathML ancestor's `lang` is skipped too, same as its own
/// element case), and a chain that bottoms out with no HTML/SVG element
/// carrying `lang` still resolves to `None`, same as "absent everywhere"
/// below.
///
/// # Deliberately out of scope
///
/// - **`xml:lang` (XML-namespace `lang`)** — the first step in the quoted
///   list, and it *would* take priority over the plain `lang` attribute.
///   Skipped because raikiri does not parse XML/XHTML documents at all yet
///   (`StyleDom::quirks_mode` doc / `resolve_case_sensitivity` doc: "raikiri
///   は現時点で HTML document のみ対象") — there is no XML-namespace
///   attribute surface to read.
/// - **`lang=""` stopping inheritance** — per the quoted algorithm, an
///   empty-string `lang` attribute is itself a *found* value ("the primary
///   language is unknown", a distinct terminal state from "no `lang`
///   attribute at all", which would keep walking to the parent). This
///   function cannot observe that distinction: [`StyleElement::attr`]'s
///   contract already collapses `foo=""` to `None` uniformly (documented on
///   that trait method, bd raikiri-spike-k5y3 — an existing accepted M1.4+
///   baseline, not something newly introduced here), so `lang=""` and "no
///   `lang` attribute" are indistinguishable at this crate's DOM boundary —
///   both fall through to the parent-element walk below. Fixing this would
///   require widening `StyleElement::attr`'s contract, which is
///   `raikiri-style`-only-change out of scope the same way k5y3 already
///   reasons about `[foo=""]` attribute-selector matching.
fn effective_language<D: StyleDom, E: StyleElement>(
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> Option<String> {
    if let Some(lang) = own_html_or_svg_lang_attribute(elem) {
        return Some(lang.to_owned());
    }
    for &ancestor_id in ancestors.iter().rev() {
        // `node`'s borrow must outlive `ancestor_elem`'s — a `.and_then`
        // chain would try to return a `&str` borrowed from a `node` that
        // drops at the end of the closure, hence the explicit `if let`
        // nesting instead of the more compact combinator chain
        // [`effective_language`]'s own doc-adjacent sibling functions use
        // where the borrow doesn't need to cross a temporary like this.
        if let Some(node) = dom.node(ancestor_id)
            && let Some(ancestor_elem) = node.as_element()
            && let Some(lang) = own_html_or_svg_lang_attribute(&ancestor_elem)
        {
            return Some(lang.to_owned());
        }
    }
    None
}

/// The **own**-attribute half of HTML LS §3.2.6.2's "determine the language
/// of a node" step (quoted in full on [`effective_language`]'s doc), gated
/// to HTML-namespace (`elem.namespace_uri() == None`, this crate's
/// established "is HTML" proxy — see [`own_explicit_direction`]'s doc) or
/// SVG-namespace elements. Any other namespace (MathML concretely, but the
/// gate is namespace-allowlist shaped so it excludes any future foreign
/// namespace equally) returns `None` regardless of whether `lang` is
/// present on the element, so [`effective_language`]'s caller falls through
/// to the ancestor walk exactly as if `lang` were absent — matching the
/// quoted algorithm's own next step ("If the node's parent element is not
/// null — Use the language of that parent element").
fn own_html_or_svg_lang_attribute<E: StyleElement>(elem: &E) -> Option<&str> {
    match elem.namespace_uri() {
        None | Some("http://www.w3.org/2000/svg") => elem.attr("lang"),
        Some(_) => None,
    }
}

/// Does `range` (one comma-separated argument of `:lang(...)`) match
/// `content_language` (the resolved [`effective_language`])? Implements RFC
/// 4647 §3.3.2 "Extended Filtering"
/// (<https://www.rfc-editor.org/rfc/rfc4647.html#section-3.3.2>, 2026-08-12
/// direct fetch), which CSS Selectors L4 §7.2 cites verbatim (bikeshed
/// source `selectors-4/Overview.bs`, same fetch as [`Direction`]'s doc —
/// the published TR page truncated before §7.2 for this crate's WebFetch
/// tool):
///
/// > The element's content language matches a language range if its content
/// > language, as represented in BCP 47 syntax, matches the given language
/// > range in an extended filtering operation per \[RFC4647\] (section
/// > 3.3.2).
///
/// RFC 4647 §3.3.2 verbatim (direct fetch of `rfc-editor.org`'s plain-text
/// rendering):
///
/// > 1. Split both the extended language range and the language tag being
/// >    compared into a list of subtags by dividing on the hyphen (%x2D)
/// >    character. Two subtags match if either they are the same when
/// >    compared case-insensitively or the language range's subtag is the
/// >    wildcard '*'.
/// > 2. Begin with the first subtag in each list. If the first subtag in
/// >    the range does not match the first subtag in the tag, the overall
/// >    match fails. Otherwise, move to the next subtag in both the range
/// >    and the tag.
/// > 3. While there are more subtags left in the language range's list:
/// >    A. If the subtag currently being examined in the range is the
/// >       wildcard ('*'), move to the next subtag in the range and
/// >       continue with the loop.
/// >    B. Else, if there are no more subtags in the language tag's list,
/// >       the match fails.
/// >    C. Else, if the current subtag in the range's list matches the
/// >       current subtag in the language tag's list, move to the next
/// >       subtag in both lists and continue with the loop.
/// >    D. Else, if the language tag's subtag is a "singleton" (a single
/// >       letter or digit, which includes the private-use subtag 'x') the
/// >       match fails.
/// >    E. Else, move to the next subtag in the language tag's list and
/// >       continue with the loop.
/// > 4. When the language range's list has no more subtags, the match
/// >    succeeds.
///
/// This function implements exactly the above (`subtags_match` = step 1's
/// per-subtag comparator). Per the CSS quote above, "the matching is
/// performed ASCII case-insensitively", which is exactly RFC4647's own
/// per-subtag rule — no separate case-folding pass needed.
///
/// # Deliberately out of scope: BCP47 canonicalization / well-formedness
///
/// CSS Selectors L4 §7.2 additionally requires the content language and the
/// range to be "canonicalized and converted to extlang form as per section
/// 4.5 of \[RFC5646\] prior to the extended filtering operation" and that
/// "language tags or ranges that are not valid do not match anything" /
/// "language ranges that are not well-formed \[...\] do not match anything".
/// Implementing BCP47 canonicalization and well-formedness validation is a
/// substantial undertaking on its own (a subtag registry / grammar checker),
/// comparable in scope to the Unicode Bidi Algorithm this task's bd
/// description explicitly permits deferring for `:dir()`'s `auto` value —
/// this function skips both and operates directly on the raw hyphen-split
/// subtag strings as written. In practice this only under-rejects (a
/// genuinely ill-formed tag like `:lang(åå)` — non-ASCII, spec says "would
/// not match" — is instead compared subtag-by-subtag and may spuriously
/// match); it never causes a spec-valid match to be missed. No known
/// real-world content in this repo's test corpus depends on the rejection
/// behavior.
pub(crate) fn language_range_matches(range: &str, content_language: &str) -> bool {
    fn subtags_match(range_subtag: &str, tag_subtag: &str) -> bool {
        range_subtag == "*" || range_subtag.eq_ignore_ascii_case(tag_subtag)
    }
    fn is_singleton(subtag: &str) -> bool {
        subtag.chars().count() == 1
    }

    let range_subtags: Vec<&str> = range.split('-').collect();
    let tag_subtags: Vec<&str> = content_language.split('-').collect();

    // Step 2: first subtag must match (range's first subtag may itself be
    // `*`, e.g. the bare wildcard range `:lang(*)` — `subtags_match` already
    // handles that).
    if !subtags_match(range_subtags[0], tag_subtags[0]) {
        return false;
    }
    let mut ri = 1;
    let mut ti = 1;

    // Step 3.
    while ri < range_subtags.len() {
        let r = range_subtags[ri];
        if r == "*" {
            ri += 1; // 3.A
            continue;
        }
        let Some(&t) = tag_subtags.get(ti) else {
            return false; // 3.B
        };
        if subtags_match(r, t) {
            ri += 1;
            ti += 1; // 3.C
            continue;
        }
        if is_singleton(t) {
            return false; // 3.D
        }
        ti += 1; // 3.E
    }
    true // Step 4.
}

/// `PseudoClass::Dir` arm of [`compound_matches`] — resolves the element's
/// **directionality** per HTML Living Standard §3.2.6.4 "The `dir`
/// attribute" (<https://html.spec.whatwg.org/multipage/dom.html#the-directionality>,
/// 2026-08-12 direct fetch) simplified to explicit `ltr`/`rtl` only:
///
/// > The directionality of an element \[...\] is either 'ltr' or 'rtl'. To
/// > compute the directionality given an element element, switch on
/// > element's dir attribute state: LTR — Return 'ltr'. RTL — Return 'rtl'.
/// > \[...\] Undefined \[...\] Otherwise — Return the parent directionality
/// > of element.
/// >
/// > To compute the parent directionality given an element element: Let
/// > parentNode be element's parent node. \[...\] If parentNode is an
/// > element, then return the directionality of parentNode. Return 'ltr'.
///
/// i.e. own `dir="ltr"`/`dir="rtl"` wins; otherwise walk up to the nearest
/// ancestor with an explicit `ltr`/`rtl` `dir`; if none exists anywhere
/// (including at the document root, which has no parent element), the
/// default is `'ltr'` — this function is total (`Direction`, not
/// `Option<Direction>`), matching the spec's own "always ltr or rtl, never
/// undetermined" shape.
///
/// CSS Selectors L4 §7.1 <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>
/// (bikeshed source, same fetch as [`Direction`]'s doc) is what motivates
/// consulting ancestors at all rather than just the own attribute (`[dir=C]`
/// would suffice for that): "the directionality of an element inherits so
/// that a child without a dir attribute will have the same directionality
/// as its closest ancestor with a valid dir attribute."
///
/// # Deliberately out of scope: `auto` state and its content-sniffing fallback
///
/// The HTML quote above elides the `Auto` state's own arm, which this
/// function folds into `Undefined`'s "return the parent directionality"
/// behavior instead of implementing — see [`Direction`]'s doc for why this
/// is a *real* behavioral divergence (Auto's own content-sniffing fallback
/// is `'ltr'` unconditionally, not the parent's directionality) and not
/// merely an implementation-order simplification. Also elided: the `bdi`
/// element and `input[type=tel]` special cases in HTML's `Undefined` arm
/// (raikiri has no notion of element-specific behavior at this layer) and
/// the shadow-tree host step of "parent directionality" (raikiri has no
/// shadow DOM).
pub(crate) fn resolve_directionality<D: StyleDom, E: StyleElement>(
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> Direction {
    if let Some(dir) = own_explicit_direction(elem) {
        return dir;
    }
    for &ancestor_id in ancestors.iter().rev() {
        // Same "explicit `if let` nesting instead of `.and_then` chain"
        // reason as [`effective_language`]'s sibling loop — `own_explicit_direction`
        // borrows from `ancestor_elem`, which itself borrows from a `node`
        // temporary that must stay alive across the call.
        if let Some(node) = dom.node(ancestor_id)
            && let Some(ancestor_elem) = node.as_element()
            && let Some(dir) = own_explicit_direction(&ancestor_elem)
        {
            return dir;
        }
    }
    Direction::Ltr
}

/// HTML's `dir` attribute LTR/RTL states only (`Undefined`/`Auto` — missing,
/// invalid, or `auto` — all collapse to `None` here; see
/// [`resolve_directionality`] doc for how the caller folds `Auto` into the
/// same "keep walking up" treatment as `Undefined`). Attribute keyword
/// matching is ASCII case-insensitive, per HTML's general treatment of
/// enumerated attribute keywords (`the dir attribute is an enumerated
/// attribute with the following keywords and states`, same fetch as
/// [`resolve_directionality`]'s doc) — same posture as this crate's other
/// HTML-enumerated-value comparisons (e.g. `elem.tag_name()`'s
/// `eq_ignore_ascii_case` in [`compound_matches`]).
///
/// # HTML-namespace-only, unlike [`effective_language`]'s `lang` reads
///
/// The same fetch [`resolve_directionality`]'s doc quotes continues,
/// immediately after the quoted algorithm:
///
/// > Since the `dir` attribute is only defined for HTML elements, it cannot
/// > be present on elements from other namespaces. Thus, elements from
/// > other namespaces always end up using the parent directionality.
///
/// so a `dir` attribute on a foreign-namespace element (e.g. inline
/// `<svg dir="rtl">`) must not be read here — it falls through to
/// [`resolve_directionality`]'s ancestor walk instead, same as if the
/// attribute were absent. `elem.namespace_uri().is_none()` is this crate's
/// established "is an HTML element" proxy — the same one
/// `resolve_case_sensitivity`'s doc documents and
/// `resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive`
/// pins for the analogous case-sensitivity divergence.
///
/// This is a *narrower* allowlist than HTML's `lang` step in the same
/// algorithm ("If the node is an HTML element **or an element in the SVG
/// namespace**, and it has a lang in no namespace attribute set" — quoted in
/// full on [`effective_language`]'s doc, gated by that function's own
/// [`own_html_or_svg_lang_attribute`] helper): both `dir` (here) and `lang`
/// gate on `elem.namespace_uri()`, but `dir` is HTML-only (1-element
/// allowlist — the "only defined for HTML elements" quote above has no SVG
/// carve-out) while `lang` is HTML-**or**-SVG (2-element allowlist). The
/// difference is allowlist *size*, not gate-vs-no-gate — do not widen this
/// function's allowlist to match [`own_html_or_svg_lang_attribute`]'s; `dir`
/// genuinely has no SVG exception in the quoted algorithm.
fn own_explicit_direction<E: StyleElement>(elem: &E) -> Option<Direction> {
    if elem.namespace_uri().is_some() {
        return None;
    }
    match elem.attr("dir") {
        Some(v) if v.eq_ignore_ascii_case("ltr") => Some(Direction::Ltr),
        Some(v) if v.eq_ignore_ascii_case("rtl") => Some(Direction::Rtl),
        _ => None,
    }
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
///
/// # 「in html document」は quirks-mode と別軸
///
/// ここでの "in html document" は CSS Selectors L4 §3.7/§6.3 が定める
/// document-**language** (HTML vs XML) 軸であり、id/class matching が使う
/// quirks-mode 軸 (`StyleQuirksMode`、CSS Selectors L4 §6.6/§6.7) とは
/// spec 上別概念 — 混同しないこと。raikiri-html は HTML5 tree builder のみで
/// XML document を生成する経路が無いため、この軸は現状 unconditionally true
/// で正しい。XML document parsing が入るときに、document-language 信号を
/// この関数へ渡す配線が必要になる (bd raikiri-spike-tqwi)。
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
/// # Cascade origin (retagged `Origin::AuthorPresentationalHint` 2026-08-11
/// — bd raikiri-spike-wo36, spec text 再確認済み)
///
/// CSS Cascading L5 §6.5 "Precedence of Non-CSS Presentational Hints"
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>, verbatim) はこの種の
/// hint を "a special-purpose author presentational hint origin between the
/// regular user origin and the author origin" — user origin と author
/// origin の間に位置する独立 origin — に置くことを定め、"Presentational
/// hints entering the cascade as author presentational hint origin rules
/// can be overridden by author-origin styles, but not by non-important
/// user-origin styles" と続ける。本関数は専用 variant
/// [`Origin::AuthorPresentationalHint`] を採る — [`cascade_rank`] はこれを
/// `(User, false) => 1` より上、`(Author, false) => 3` より下に置く (bd
/// raikiri-spike-pdta で [`Origin::User`] 挿入後の値 — [`cascade_rank`] doc
/// 参照)。この rank 差は `beats` の tuple compare `(rank, specificity, source_order)` の
/// **第一要素**なので、真の UA-origin rule には specificity/source_order を
/// 問わず常に勝ち、real author-origin 宣言 (stylesheet rule でも inline
/// style でも) には specificity/source_order を問わず常に負ける — かつて
/// (2026-08-10 retag 時点、旧版は hint も real 宣言も同じ `Origin::Author`
/// に tag していた) は後者の保証を「hint の specificity を 0 に固定し、
/// real 宣言が zero-specificity かつ stylesheet 先頭 rule の場合に限り
/// 発生する exact tie を push 順序 (hint を先に push) で決着させる」という
/// 同一 origin 内 tie-break に依存していた — 3rd tier 導入によりその依存は
/// 解消され、origin rank だけで無条件に決着する。[`Origin::User`] の挿入
/// (bd raikiri-spike-pdta) はこの結論を変えない — hint の rank は挿入後も
/// 依然として real `Author` rank と等しくなることが無い (`AuthorPresentationalHint`
/// と `Author` は常に隣接する別 rank 値のまま、[`cascade_rank`] doc の rank
/// 表参照) ため、[`collect_cascaded`] が今も stylesheet rule matching /
/// inline style より先にこの関数を push する呼び出し順は残っているが、
/// 上記の通りもう correctness の必要条件ではない (無害な残置、pdta で
/// re-verify 済み)。
///
/// テスト
/// `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
/// はこの「specificity を問わず real author 宣言が勝つ」性質を、かつては
/// exact-tie 経由で、今は origin rank 差で直接 exercise する ([`Origin::User`]
/// 挿入後も rank 差の大小関係は変わらないため、この test は無変更で pin
/// し続ける)。
///
/// 残る `Origin::User` no-producer 残差 (旧: bd raikiri-spike-wo36 item 4): bd
/// raikiri-spike-pdta で raikiri-style 内の [`Origin::User`] variant 自体は
/// 追加済み ([`cascade_rank`] は 4-tier 化済み) だが、consumer が渡す
/// `extra_stylesheets` を実際に [`Origin::User`] へ route する producer は
/// まだ無い — 今も `StylesheetKind::Author` 経由で [`Origin::Author`] として
/// 届く ([`crate::ruletree`] module doc 参照)。spec の完全な順序では hint は
/// 「user origin より強い」はずだが、本実装は user stylesheet 宣言を hint
/// より強い [`Origin::Author`] rank に一律 fold している — user stylesheet
/// が img の width/height を上書きできるという結果自体は spec と一致するが
/// (`Author` rank は hint より常に上)、独立した User origin へ実際に route
/// すれば hint が真の author 宣言だけに overridable になる、というモデルの
/// 精度としては不完全なまま。raikiri-traits 側の `StylesheetKind` に
/// 独立 variant を追加し raikiri-html で retag し、umbrella 側の
/// `stylesheet_kind_to_origin` を拡張する必要がある genuine multi-crate diff
/// (raikiri-style 単体では完結しない) のため bd raikiri-spike-d7h3 に
/// 切り出し済み (wall/traits 経路)。
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
            Origin::AuthorPresentationalHint,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
        ));
    }
    if let Some(height) = elem.attr("height").and_then(parse_html_dimension_value) {
        decls.push((
            PropertyValue::Height(LengthOrAuto::Length(height)),
            false,
            Origin::AuthorPresentationalHint,
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
    // - rank 高い方が勝つ (順序と正確な値は `cascade_rank` doc 参照 — bd
    //   raikiri-spike-wo36 で UA/Author の 2 段から
    //   UA/AuthorPresentationalHint/Author の 3 段に拡張、bd raikiri-spike-pdta
    //   で UA/User/AuthorPresentationalHint/Author の 4 段に拡張済み)
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
        | PropertyValue::BoxSizing(_)
        // `overflow-x`/`overflow-y`/`overflow` join this arm — CSS Overflow 3
        // §3.1's cross-axis coupling (`resolve_overflow`) depends only on the
        // *other axis of the same node*, never on the inheritance parent, so
        // there is nothing for this function (phase 2) to resolve. It is
        // applied in phase 3 instead (`crate::page::absolutize_in_page_context`,
        // mirroring the element path's `SpecifiedValues::absolutize_with`).
        // (raikiri-spike-cmd3)
        | PropertyValue::OverflowX(_)
        | PropertyValue::OverflowY(_)
        | PropertyValue::Overflow(_)) => v,
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
        // CSS Overflow 3 §3.1 overflow-x/overflow-y physical longhand
        // (raikiri-spike-cmd3)。non-inherited、per-axis winner を staging の
        // `overflow.x`/`overflow.y` へ直接代入。cross-axis の computed-value
        // coupling (`resolve_overflow`) はここでは**適用しない** —
        // `target.overflow` は winner 適用の途中経過であり、まだ他方の axis の
        // 最終 winner を反映し終えていない可能性がある。coupling は全 winner
        // 適用後の phase 3 (`SpecifiedValues::finalize` → `absolutize_with`)
        // でのみ解決する (border style→width gating と同じ順序、
        // [`resolve_overflow`] doc 参照)。
        PropertyValue::OverflowX(v) => target.overflow.x = v,
        PropertyValue::OverflowY(v) => target.overflow.y = v,
        // `overflow` shorthand fall-through。sibling `PropertyValue::Padding`
        // arm と同じく **safety net ではない** — 到達すれば 2 longhand winner
        // を一括で破壊し spec と食い違う。cascade 経路では unreachable
        // (`expand_shorthand_into` が 2 longhand に展開する)。詳細な framing
        // とその unreachability の compile-time 強制は `Margin` arm の
        // comment 参照。(raikiri-spike-cmd3)
        PropertyValue::Overflow(pair) => target.overflow = pair,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::INITIAL_FONT_SIZE_PX;
    use crate::property::CssColor;
    use crate::property::DisplayValue;
    use crate::property::{
        Border, BorderColor, BorderStyle, Length, LengthOrAuto, OverflowValue, OverflowXY, Sides,
    };
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

    /// bd raikiri-spike-tqwi acceptance: quirks-mode 3 状態 × class-selector
    /// case variation の regression matrix. CSS Selectors L4 class-html
    /// (<https://www.w3.org/TR/selectors-4/#class-html>, verbatim): "When
    /// matching against a document which is in quirks mode, class names must
    /// be matched ASCII case-insensitively; class selectors are otherwise
    /// case-sensitive". `NoQuirks`'s different-case branch overlaps
    /// `class_selector_is_case_sensitive` above (kept as its own smaller,
    /// standalone regression test); this table adds `LimitedQuirks` /
    /// `Quirks` plus a same-case control per mode so a regression that stops
    /// matching entirely (rather than over-folding) would also be caught.
    #[test]
    fn class_selector_case_sensitivity_across_quirks_modes() {
        struct Case {
            mode: StyleQuirksMode,
            element_class: &'static str,
            expect_match: bool,
        }
        let cases = [
            // NoQuirks (standards mode): always case-sensitive.
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_class: "foo",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_class: "FOO",
                expect_match: false,
            },
            // LimitedQuirks ("almost standards"): distinct DOM Standard dfn
            // from "quirks mode" — must NOT fold, same as NoQuirks.
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_class: "foo",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_class: "FOO",
                expect_match: false,
            },
            // Quirks (full quirks mode): ASCII case-insensitive fold.
            Case {
                mode: StyleQuirksMode::Quirks,
                element_class: "foo",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::Quirks,
                element_class: "FOO",
                expect_match: true,
            },
        ];

        for case in cases {
            let mut doc = TestDoc::new();
            doc.quirks_mode = case.mode;
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, ".foo { font-weight: bold }");
            let p = doc.push_element(0, "p", None);
            doc.set_attr(p, "class", case.element_class);

            let tree = build_rule_tree(&doc);
            let r = cascade(&doc, &tree).expect("cascade Ok");
            let matched = r.computed[p].font_weight == 700.0;
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                matched, case.expect_match,
                "mode={:?} element_class={:?}: expected match={}",
                case.mode, case.element_class, case.expect_match
            );
        }
    }

    /// `has_class_ascii_case_insensitive`'s empty-query guard. A real CSS
    /// class selector can never lexically produce an empty class name, so
    /// `compound_matches` can't reach this branch — hence a direct
    /// call on `TestElementRef`, mirroring
    /// `match_complex_selector_list_rejects_unsupported_component_via_safety_net`
    /// below.
    #[test]
    fn has_class_ascii_case_insensitive_empty_query_is_false() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo");
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        assert!(!elem.has_class_ascii_case_insensitive(""));
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

    /// bd raikiri-spike-tqwi acceptance: quirks-mode 3 状態 × id-selector case
    /// variation の regression matrix — [`class_selector_case_sensitivity_across_quirks_modes`]
    /// の id-selector 版。CSS Selectors L4 id-selectors
    /// (<https://www.w3.org/TR/selectors-4/#id-selectors>, verbatim): "When
    /// matching against a document which is in quirks mode, IDs must be
    /// matched ASCII case-insensitively; ID selectors are otherwise
    /// case-sensitive".
    #[test]
    fn id_selector_case_sensitivity_across_quirks_modes() {
        struct Case {
            mode: StyleQuirksMode,
            element_id: &'static str,
            expect_match: bool,
        }
        let cases = [
            // NoQuirks (standards mode): always case-sensitive.
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_id: "header",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_id: "HEADER",
                expect_match: false,
            },
            // LimitedQuirks ("almost standards"): distinct DOM Standard dfn
            // from "quirks mode" — must NOT fold, same as NoQuirks.
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_id: "header",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_id: "HEADER",
                expect_match: false,
            },
            // Quirks (full quirks mode): ASCII case-insensitive fold.
            Case {
                mode: StyleQuirksMode::Quirks,
                element_id: "header",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::Quirks,
                element_id: "HEADER",
                expect_match: true,
            },
        ];

        for case in cases {
            let mut doc = TestDoc::new();
            doc.quirks_mode = case.mode;
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, "#header { color: red }");
            let div = doc.push_element(0, "div", None);
            doc.set_attr(div, "id", case.element_id);

            let tree = build_rule_tree(&doc);
            let r = cascade(&doc, &tree).expect("cascade Ok");
            let matched = r.computed[div].color == RED;
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                matched, case.expect_match,
                "mode={:?} element_id={:?}: expected match={}",
                case.mode, case.element_id, case.expect_match
            );
        }
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
        //
        // This is an **intentional accepted-baseline pin, not a silently
        // tolerated bug**: the decision to accept this CSS Selectors L4
        // divergence as a permanent M1.4+ simplification is formally
        // recorded in bd raikiri-spike-k5y3 (g04 accept/reject category (c),
        // "intentional stricter") — see `StyleElement::attr`'s doc comment
        // and that decision for the full rationale.
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
    fn attribute_exact_match_selector_does_not_match_empty_value_attr() {
        // Same root cause as
        // `attribute_exists_selector_does_not_match_empty_value_attr` above,
        // pinned separately because it goes through a different
        // `compound_matches` arm (`Component::AttributeInNoNamespace`,
        // not `..Exists`): the `StyleElement::attr` contract ("Empty string
        // is normalised to `None`", style_dom.rs doc) collapses `foo=""`
        // into `None` before the with-value arm's `match elem.attr(...) {
        // Some(..) => .., None => false }` ever runs, so it takes the
        // `None => false` branch regardless of the selector's own value
        // operand. Per CSS Selectors L4
        // (<https://www.w3.org/TR/selectors-4/#attribute-selectors>),
        // `[data-foo=""]` should match an element whose `data-foo` value is
        // exactly the empty string — it does not here, for the same
        // documented reason `[data-foo]` doesn't.
        //
        // This is an **intentional accepted-baseline pin, not a silently
        // tolerated bug**: the decision to accept this CSS Selectors L4
        // divergence as a permanent M1.4+ simplification is formally
        // recorded in bd raikiri-spike-k5y3 (g04 accept/reject category (c),
        // "intentional stricter") — see `StyleElement::attr`'s doc comment
        // and that decision for the full rationale.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"\"] { color: red }");
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

    // --- descendant / child combinator (bd raikiri-spike-flln.2) ---
    //
    // CSS Selectors L4 descendant combinator
    // (<https://www.w3.org/TR/selectors-4/#descendant-combinators>, verbatim:
    // "A selector of the form A B represents an element B that is an
    // arbitrary descendant of some ancestor element A") and child combinator
    // (<https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim: "A
    // child combinator describes a childhood relationship between two
    // elements") — see `match_combinator_chain`'s "Spec provenance note" doc
    // section for how this verbatim text was confirmed.

    #[test]
    fn descendant_combinator_applies_declaration_to_direct_child() {
        // Acceptance (bd raikiri-spike-flln.2): `.chapter h2` applied to
        // `<div class="chapter"><h2>...</h2></div>`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "chapter");
        let h2 = doc.push_element(div, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[h2].color, RED);
    }

    #[test]
    fn descendant_combinator_applies_to_arbitrary_depth_descendant() {
        // "arbitrary descendant" (spec verbatim above) — must match even
        // through an intermediate <section> that itself matches neither
        // side of the selector.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "chapter");
        let section = doc.push_element(div, "section", None);
        let h2 = doc.push_element(section, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h2].color, RED,
            "descendant combinator must match through an intermediate non-matching ancestor"
        );
    }

    #[test]
    fn descendant_combinator_does_not_match_outside_the_subtree() {
        // Negative case: an <h2> that is not a descendant of any
        // `.chapter` must not pick up the declaration.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None); // no class="chapter"
        let h2 = doc.push_element(div, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[h2].color, ComputedValues::initial().color);
    }

    #[test]
    fn child_combinator_applies_declaration_to_direct_child_only() {
        // Acceptance (bd raikiri-spike-flln.2): `ol > li` applies to a
        // direct `<li>` child of `<ol>`, but NOT to a grandchild `<li>`
        // reached through an intervening `<ul>` (`<ol><li><ul><li>...`).
        //
        // Uses `background-color`, not `color`: `color` is an inherited
        // property (CSS Cascading L4 §5.2 inheritance) — using it here would
        // let the *direct* `<li>` match's computed value leak onto the
        // grandchild via ordinary inheritance (through the intervening
        // `<ul>`), producing a false pass regardless of whether the child
        // combinator itself correctly rejects the grandchild.
        // `background-color` is not inherited (CSS Backgrounds 3 §2.2), so a
        // red grandchild here can only mean the combinator matched it
        // directly.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "ol > li { background-color: red }");
        let ol = doc.push_element(0, "ol", None);
        let direct_li = doc.push_element(ol, "li", None);
        let ul = doc.push_element(direct_li, "ul", None);
        let grandchild_li = doc.push_element(ul, "li", None);
        // Root-level `<li>` with no `<ol>` ancestor at all (its
        // `ancestor_path` is empty, since the document root itself is not
        // an `Element`) — exercises `match_combinator_chain`'s
        // `Combinator::Child => ancestors.split_last() => None => false`
        // arm, distinct from the "wrong parent" case covered by
        // `grandchild_li` above (there `ancestors.split_last()` succeeds
        // but the resolved parent fails `compound_matches`).
        let orphan_li = doc.push_element(0, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[direct_li].background_color, RED,
            "ol > li must match the direct <li> child of <ol>"
        );
        assert_eq!(
            r.computed[grandchild_li].background_color,
            ComputedValues::initial().background_color,
            "ol > li must NOT match a grandchild <li> reached through an intervening <ul>"
        );
        assert_eq!(
            r.computed[orphan_li].background_color,
            ComputedValues::initial().background_color,
            "ol > li must NOT match an <li> with no ancestor at all"
        );
    }

    #[test]
    fn next_sibling_combinator_does_not_match_parent_child_relationship() {
        // bd raikiri-spike-flln.3: `div + p` requires `div`/`p` to be
        // *siblings* (CSS Selectors L4 adjacent-sibling-combinators,
        // "share the same parent"). Here `p` is instead a *child* of
        // `div` — the ancestor relationship must NOT satisfy the sibling
        // combinator, even though `div` is literally `ancestors.last()`.
        // Directly exercises `match_combinator_chain`'s `NextSibling` arm
        // (this test predates flln.3 as
        // `match_combinator_chain_rejects_unsupported_combinator_via_safety_net`,
        // when `+` fell through the `_ => false` safety net for a different
        // reason — repurposed now that `+` is supported).
        let list = crate::parse_selector_list("div + p").expect("selector parses");
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", None);
        let p = doc.push_element(div, "p", None);
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        assert_eq!(
            match_complex_selector_list(
                &list,
                &doc,
                &elem,
                StyleNodeId::new(p as u64),
                &[StyleNodeId::new(div as u64)],
                StyleQuirksMode::NoQuirks,
            ),
            None,
            "div + p must not match a p that is div's child, not its sibling"
        );
    }

    #[test]
    fn descendant_and_child_combinator_are_distinguished_on_the_same_grandchild() {
        // Same grandchild `<li>` as above, matched instead by a descendant
        // (space) combinator on `ol` — must match, unlike the child (`>`)
        // combinator case, directly exercising "descendant と child の区別"
        // called out in the acceptance criteria. `background-color` again
        // (see the sibling test above) so a match is provably direct, not
        // inherited from the also-matching direct `<li>`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "ol li { background-color: red }");
        let ol = doc.push_element(0, "ol", None);
        let direct_li = doc.push_element(ol, "li", None);
        let ul = doc.push_element(direct_li, "ul", None);
        let grandchild_li = doc.push_element(ul, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[direct_li].background_color, RED);
        assert_eq!(
            r.computed[grandchild_li].background_color, RED,
            "ol li (descendant combinator) must match the grandchild <li> too"
        );
    }

    #[test]
    fn chained_descendant_and_child_combinator_matches_spec_example() {
        // CSS Selectors L4 child-combinators
        // (<https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim
        // example — see `match_combinator_chain`'s "Spec provenance note"
        // for how this text was confirmed): `div ol>li p` "represents a p
        // element that is a descendant of an li element; the li element
        // must be the child of an ol element; the ol element must be a
        // descendant of a div".
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div ol>li p { color: red }");
        let div = doc.push_element(0, "div", None);
        // `ol` is a descendant of `div`, not a direct child — exercises the
        // "arbitrary descendant" half of the chained selector too.
        let wrapper = doc.push_element(div, "section", None);
        let ol = doc.push_element(wrapper, "ol", None);
        let li = doc.push_element(ol, "li", None);
        // `p` is a descendant of `li`, not a direct child.
        let span = doc.push_element(li, "span", None);
        let p = doc.push_element(span, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].color, RED);
    }

    #[test]
    fn chained_descendant_and_child_combinator_rejects_wrong_child_parent() {
        // Same shape as the spec example above, but `li`'s parent is `ul`
        // instead of `ol` — the `>` (child) constraint must reject this
        // even though every other part of the chain still lines up.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div ol>li p { color: red }");
        let div = doc.push_element(0, "div", None);
        let ul = doc.push_element(div, "ul", None); // not `ol`
        let li = doc.push_element(ul, "li", None);
        let p = doc.push_element(li, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].color, ComputedValues::initial().color);
    }

    // ── Sibling combinators (bd raikiri-spike-flln.3) ──

    #[test]
    fn adjacent_sibling_combinator_applies_only_to_immediately_following_sibling() {
        // bd raikiri-spike-flln.3 acceptance: `h2 + p` applies to the `<p>`
        // immediately following an `<h2>`, but NOT to a second/third `<p>`
        // further along — CSS Selectors L4 next-sibling combinator
        // (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>
        // §14.3, "match_combinator_chain" doc's verbatim quote). Elements
        // are pushed at the **document root** (parent id `0`, no wrapping
        // `<div>`) deliberately — `ancestor_path` only ever contains
        // Element-kind ids, so a root-level sibling pair exercises
        // `match_combinator_chain`'s `ancestors.last() == None →
        // dom.root_id()` fallback; a wrapping element would hide a bug in
        // that fallback entirely.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red }");
        let _h2 = doc.push_element(0, "h2", None);
        let p1 = doc.push_element(0, "p", None); // immediately follows h2
        let p2 = doc.push_element(0, "p", None); // follows p1, not h2
        let p3 = doc.push_element(0, "p", None); // follows p2, not h2

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p1].background_color, RED,
            "h2 + p must match the p immediately following h2"
        );
        assert_eq!(
            r.computed[p2].background_color,
            ComputedValues::initial().background_color,
            "h2 + p must NOT match the second p (not immediately after h2)"
        );
        assert_eq!(
            r.computed[p3].background_color,
            ComputedValues::initial().background_color,
            "h2 + p must NOT match the third p (not immediately after h2)"
        );
    }

    #[test]
    fn general_sibling_combinator_applies_to_every_following_sibling() {
        // bd raikiri-spike-flln.3 acceptance: `h2 ~ p` applies to every
        // `<p>` that follows an `<h2>`, not just the immediate one — CSS
        // Selectors L4 general-sibling combinator
        // (<https://www.w3.org/TR/selectors-4/#general-sibling-combinators>
        // §14.4). Same root-level layout as the adjacent-sibling test above
        // (same rationale — exercises the `ancestors.last() == None`
        // fallback).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 ~ p { background-color: red }");
        let _h2 = doc.push_element(0, "h2", None);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let p3 = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p1].background_color, RED,
            "h2 ~ p must match the 1st following p"
        );
        assert_eq!(
            r.computed[p2].background_color, RED,
            "h2 ~ p must match the 2nd following p"
        );
        assert_eq!(
            r.computed[p3].background_color, RED,
            "h2 ~ p must match the 3rd following p"
        );
    }

    #[test]
    fn adjacent_and_general_sibling_combinator_are_distinguished_on_the_same_dom() {
        // bd raikiri-spike-flln.3 acceptance, literal form: both `h2 + p`
        // and `h2 ~ p` active on the same `<h2><p><p><p>` DOM, using two
        // independent non-inherited properties (`background-color`, CSS
        // Backgrounds 3 §2.2; `box-sizing`, CSS Box Sizing dfn "Inherited:
        // no") so each combinator's reach is independently observable on
        // the same elements.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "h2 + p { background-color: red } h2 ~ p { box-sizing: border-box }",
        );
        let _h2 = doc.push_element(0, "h2", None);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let p3 = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // `+` (background-color): only p1.
        assert_eq!(r.computed[p1].background_color, RED);
        assert_eq!(
            r.computed[p2].background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(
            r.computed[p3].background_color,
            ComputedValues::initial().background_color
        );
        // `~` (box-sizing): all three.
        assert_eq!(
            r.computed[p1].box_sizing,
            crate::property::BoxSizing::BorderBox
        );
        assert_eq!(
            r.computed[p2].box_sizing,
            crate::property::BoxSizing::BorderBox
        );
        assert_eq!(
            r.computed[p3].box_sizing,
            crate::property::BoxSizing::BorderBox
        );
    }

    #[test]
    fn sibling_combinator_ignores_non_element_nodes_between_siblings() {
        // CSS Selectors L4 next-sibling combinator, verbatim: "Non-element
        // nodes (e.g. text between elements) are ignored when considering
        // the adjacency of elements"
        // (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>).
        // A text node is pushed to the *root* (same parent as `h2`/`p`)
        // between them — `TestDoc::push_text(parent, ..)` appends to
        // `parent`'s children list in call order, so pushing it between the
        // `h2` and `p` pushes below makes it a genuine root-level sibling
        // positioned between them, not a descendant of either. `h2 + p`
        // must still match `p` despite this — i.e.
        // `match_combinator_chain`'s `NextSibling` arm
        // (`immediate_preceding_sibling`) must skip the non-Element
        // `child_ids` entry rather than treating the text node as "the"
        // immediately preceding sibling (which would make `p` NOT
        // immediately follow `h2` from an all-nodes perspective).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red }");
        let _h2 = doc.push_element(0, "h2", None);
        doc.push_text(0, "root-level text node, sibling of h2 and p, between them");
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_color, RED,
            "h2 + p must match p despite the text node inside h2 (not a sibling at all) \
             and must not be confused by non-element nodes in general"
        );
    }

    #[test]
    fn general_sibling_combinator_skips_a_leading_non_element_candidate() {
        // Same CSS Selectors L4 "non-element nodes... are ignored" rule as
        // `sibling_combinator_ignores_non_element_nodes_between_siblings`
        // above, but for `~` (general sibling) rather than `+` (adjacent
        // sibling) — these exercise different code paths:
        // `PendingCandidates::LaterSibling`'s new resumable scan loop vs
        // `PendingCandidates::NextSibling`'s single-shot
        // `immediate_preceding_sibling`, for the same kind()-based skip
        // (`TestDoc` always reports `is_in_document() == true`; it has no
        // inert/`<template>`-descendant node concept, so this doesn't
        // separately discriminate `is_in_document_element`'s
        // `is_in_document()` conjunct from its `kind() == Element` one). A
        // root-level text node is pushed *before* `h2` (not between `h2`
        // and `p` — `~`'s candidate scan walks the parent's children
        // forward from the start, so the non-element candidate must be
        // reached *before* the eventual matching candidate to exercise the
        // "skip, keep scanning" branch rather than the "reached
        // `current_id`, stop" one).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 ~ p { background-color: red }");
        doc.push_text(0, "root-level text node, sibling of h2 and p, before both");
        let _h2 = doc.push_element(0, "h2", None);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].background_color, RED,
            "h2 ~ p must match p despite a non-element sibling preceding h2 in \
             the same child list"
        );
    }

    #[test]
    fn later_sibling_choice_point_resumes_live_iterator_across_backtrack() {
        // Discriminator for `PendingCandidates::LaterSibling`'s *live,
        // resumable* `D::ChildIter` (bd raikiri-spike-8r16 review probe,
        // reviewer:spec) - not covered by any existing test: doubling `~`
        // is required to exercise one `LaterSibling` choice point being
        // popped-into-and-resumed by a stack.pop() backtrack from a
        // *different, nested* `LaterSibling` choice point (a single `~`
        // combined with `>`/` ` can't discriminate this, since sibling
        // candidates all share the same ancestors, so a Child/Descendant
        // check after a sibling jump can't distinguish "resume mid-scan"
        // from "rescan from the top").
        //
        // Children of the shared parent, document order: b1(.b), a(.a),
        // b2(.b), t(.t). Selector `.a ~ .b ~ .t` requires some `.b` that
        // precedes `t`, itself preceded by some `.a`.
        //
        // b1 is the *first* `.b` candidate tried for `t`'s `~` frame - but
        // b1's own nested `~` scan for `.a` immediately hits its `stop_at`
        // (b1 is the very first child), so that inner frame is exhausted
        // with zero candidates and pops immediately. Correctness requires
        // the *outer* frame (scanning for `.b` before `t`) to resume its
        // live iterator at `a` next (not restart at b1 - that would loop
        // forever / re-fail identically - and not skip past `a` straight to
        // `b2`, which would only find the wrong, but still spec-correct-
        // looking, match via a different `.b` and hide a real skip bug).
        // The only `.b` with a valid `.a` before it is b2 (via `a`), so a
        // match requires both correct resume *and* correct non-skip.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".a ~ .b ~ .t { background-color: red }");
        let b1 = doc.push_element(0, "div", None);
        doc.set_attr(b1, "class", "b");
        let a = doc.push_element(0, "div", None);
        doc.set_attr(a, "class", "a");
        let b2 = doc.push_element(0, "div", None);
        doc.set_attr(b2, "class", "b");
        let t = doc.push_element(0, "div", None);
        doc.set_attr(t, "class", "t");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[t].background_color, RED,
            ".a ~ .b ~ .t must match t via b2 (preceded by a), after b1's own \
             nested .a search (immediately empty) is backtracked past - this \
             requires the LaterSibling choice point's live child iterator to \
             resume correctly rather than restart or skip"
        );
    }

    #[test]
    fn sibling_combinator_does_not_match_preceding_element() {
        // Order matters: CSS Selectors L4 requires the left compound's
        // element to *precede* the right compound's element. A `<p>` placed
        // BEFORE the `<h2>` must not satisfy `h2 + p` / `h2 ~ p` when
        // matching is attempted from that earlier `<p>`'s perspective.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red } h2 ~ p { color: red }");
        let p_before = doc.push_element(0, "p", None);
        let _h2 = doc.push_element(0, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p_before].background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(r.computed[p_before].color, ComputedValues::initial().color);
    }

    #[test]
    fn sibling_combinator_applies_under_a_non_root_parent() {
        // Same as the acceptance tests above but wrapped in a `<div>`
        // parent, so `ancestors` is non-empty when the sibling combinator
        // arms run — exercises the `ancestors.last() == Some(parent)` branch
        // (as opposed to the root-level tests' `None → root_id()` fallback
        // branch) of `match_combinator_chain`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red }");
        let wrap = doc.push_element(0, "div", None);
        let _h2 = doc.push_element(wrap, "h2", None);
        let p = doc.push_element(wrap, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].background_color, RED);
    }

    #[test]
    fn sibling_combinator_composes_with_child_combinator_further_left() {
        // Mixed chain, sibling-then-ancestor direction: `.x > .y ~ .z`.
        // `.z` and `.y` are siblings (share parent `.x`); `.y` must in turn
        // be a direct child of `.x`. Exercises the "sibling jump keeps
        // `ancestors` unchanged, so a further-left Child/Descendant combinator
        // composes for free" path documented on
        // `is_supported_selector_list` (bd raikiri-spike-flln.3).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".x > .y ~ .z { background-color: red }");
        let x = doc.push_element(0, "div", None);
        doc.set_attr(x, "class", "x");
        let y = doc.push_element(x, "div", None);
        doc.set_attr(y, "class", "y");
        let z = doc.push_element(x, "div", None); // sibling of y, child of x
        doc.set_attr(z, "class", "z");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[z].background_color, RED);
    }

    #[test]
    fn child_combinator_composes_with_sibling_combinator_further_left() {
        // Mixed chain, ancestor-then-sibling direction: `.x ~ .y > .z`.
        // `.z`'s parent is `.y`; `.y` must in turn have a preceding sibling
        // `.x` (sharing `.y`'s own parent). Exercises the opposite
        // composition from the test above — after the `Child` jump to `.y`,
        // the `ancestors` slice `match_from_element` carries onward is
        // already `.y`'s own ancestor chain, so `.last()` correctly resolves
        // to `.y`'s parent for the `LaterSibling` step (see
        // `match_combinator_chain`'s "親の解決" doc note).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".x ~ .y > .z { background-color: red }");
        let container = doc.push_element(0, "div", None);
        let x = doc.push_element(container, "div", None);
        doc.set_attr(x, "class", "x");
        let y = doc.push_element(container, "div", None); // sibling of x
        doc.set_attr(y, "class", "y");
        let z = doc.push_element(y, "div", None); // child of y
        doc.set_attr(z, "class", "z");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[z].background_color, RED);
    }

    #[test]
    fn descendant_combinator_selector_specificity_includes_ancestor_compound() {
        // Specificity of a complex selector accounts for every compound in
        // the chain, not just the rightmost one matched against `elem` —
        // `.chapter h2` (0,1,1,0) must beat a plain `h2` (0,0,0,1) rule
        // targeting the same element, CSS Cascading L4 §6.1 sort criterion
        // (b). Both rules are given later source order for the loser so the
        // win is attributable to specificity, not source order.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 { color: blue } .chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "chapter");
        let h2 = doc.push_element(div, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h2].color, RED,
            ".chapter h2 must win over plain h2 on specificity"
        );
    }

    #[test]
    fn descendant_combinator_after_backtracking_past_a_sibling_subtree() {
        // Regression for the `ancestor_path` depth-truncation technique in
        // `collect_cascaded`: after the DFS finishes a `.wrap` subtree and
        // returns to process a *sibling* `.target`, `.target`'s own
        // ancestor_path must not still contain anything pushed while
        // visiting the sibling's subtree. `.wrap p` must not leak onto
        // `.target`'s child.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".wrap p { color: red }");
        let root = doc.push_element(0, "div", None);
        let wrap = doc.push_element(root, "div", None);
        doc.set_attr(wrap, "class", "wrap");
        let wrapped_p = doc.push_element(wrap, "p", None);
        let target = doc.push_element(root, "div", None);
        doc.set_attr(target, "class", "target"); // sibling of `wrap`, NOT `.wrap`
        let p_under_target = doc.push_element(target, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // Positive control (Codex §8.3 finding): without this, a matcher
        // that never matches `.wrap p` anywhere would also make the
        // negative assertion below pass vacuously — assert the rule
        // actually fired where it should have before asserting it didn't
        // leak where it shouldn't.
        assert_eq!(
            r.computed[wrapped_p].color, RED,
            ".wrap p must match its own direct target under .wrap"
        );
        assert_eq!(
            r.computed[p_under_target].color,
            ComputedValues::initial().color,
            "p under .target must not match .wrap p just because a sibling .wrap subtree was visited earlier"
        );
    }

    #[test]
    fn descendant_retry_past_a_failed_child_combinator_candidate_is_required() {
        // Regression pinned by 3 independently-converging reviewer lenses
        // (spec/quality/debt) on this branch's first draft, which had a
        // *false* doc-comment claim on `match_combinator_chain` that the
        // `Combinator::Descendant` retry loop is provably redundant for an
        // ancestor-chain-only combinator subset. That's only true when
        // *every* subsequent combinator is also `Descendant` (a free
        // existential search over a strictly-growing superset as you pick
        // a nearer anchor). It breaks the moment a `Combinator::Child` sits
        // further left: `Child` pins one *specific* element
        // (`ancestors.split_last()`), not "any element in the remaining
        // set" — different `Descendant` anchor choices pin genuinely
        // different elements, not nested subsets of the same free search.
        // See `match_combinator_chain`'s doc (2026-08-12 correction) for
        // the full argument this test exists to pin.
        //
        // Selector `.x > .y .target` against
        // `G(.x) -> F(.y) -> M(no class) -> C(.y) -> elem(.target)`:
        // the *nearest* `.y` candidate is `C`, but `Child` forces checking
        // `C`'s immediate parent `M` specifically, which lacks `.x` — a
        // confirmed dead end. Only the *farther* `.y` candidate `F` works,
        // because `Child` then forces checking `F`'s immediate parent `G`,
        // which does have `.x`. Without the retry (stopping at `C`'s
        // failure), this selector would silently stop matching.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".x > .y .target { background-color: red }");
        let g = doc.push_element(0, "div", None);
        doc.set_attr(g, "class", "x");
        let f = doc.push_element(g, "div", None);
        doc.set_attr(f, "class", "y");
        let m = doc.push_element(f, "div", None); // no class — the dead-end Child target for C
        let c = doc.push_element(m, "div", None);
        doc.set_attr(c, "class", "y"); // nearest .y candidate, but a dead end via Child
        let target = doc.push_element(c, "div", None);
        doc.set_attr(target, "class", "target");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[target].background_color, RED,
            ".x > .y .target must match via the farther .y candidate (F) after the \
             nearer one (C) fails Child's fixed-parent check — the retry is required"
        );
    }

    #[test]
    fn match_complex_selector_list_rejects_unsupported_component_via_safety_net() {
        // pseudo-class components never reach `match_complex_selector_list`
        // in the real pipeline — `ruletree.rs`'s `is_supported_selector_list`
        // drops any rule containing one at `add_stylesheet` time (pinned by
        // `ruletree::tests::pseudo_class_selector_still_dropped`). This test
        // calls `match_complex_selector_list` directly — both it and
        // `parse_selector_list` are reachable from this `#[cfg(test)] mod
        // tests` (`use super::*` / `crate::parse_selector_list`) — to
        // exercise `compound_matches`'s `_ => false` safety-net arm
        // defensively, per its own doc comment. `p:hover` has no combinator,
        // so `ancestors`/`elem_id` are irrelevant here — `&[]` / `p`'s own id
        // (bd raikiri-spike-flln.2 renamed this test alongside the function
        // and added `dom`/`ancestors` args; bd raikiri-spike-flln.3 added
        // the `elem_id` arg the current signature requires).
        let list = crate::parse_selector_list("p:hover").expect("selector parses");
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", None);
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            match_complex_selector_list(
                &list,
                &doc,
                &elem,
                StyleNodeId::new(p as u64),
                &[],
                StyleQuirksMode::NoQuirks,
            ),
            None,
            "NonTSPseudoClass component must fall through the safety net"
        );
    }

    // ── `:lang()` / `:dir()` (bd raikiri-spike-flln.6) ──

    /// bd acceptance: `:lang(ja) { font-family: serif-ja }` applies to an
    /// element that has `lang="ja"` **directly on itself**.
    #[test]
    fn lang_matches_own_lang_attribute() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "lang", "ja");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "serif-ja");
    }

    /// bd acceptance (the actual "top gap" this task closes): `:lang(ja)`
    /// must apply to an element with **no lang attribute of its own**, whose
    /// language is inherited from an ancestor (`<html lang="ja">` in the bd
    /// description's own example) — the ancestor-walk this task reuses from
    /// bd raikiri-spike-flln.2's descendant/child combinator matching.
    #[test]
    fn lang_matches_via_ancestor_inherited_language() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "ja");
        let body = doc.push_element(html, "body", None);
        let p = doc.push_element(body, "p", None); // no lang of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_family[0].to_string(),
            "serif-ja",
            ":lang(ja) must match an element with no lang attribute of its \
             own when an ancestor carries lang=\"ja\""
        );
    }

    /// Negative counterpart of the two tests above: neither the element nor
    /// any ancestor carries a `lang` attribute at all — `:lang(ja)` must not
    /// match (no content language to compare against, CSS Selectors L4 §7.2
    /// — see `effective_language`'s doc).
    #[test]
    fn lang_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
        let html = doc.push_element(0, "html", None); // no lang
        let body = doc.push_element(html, "body", None); // no lang
        let p = doc.push_element(body, "p", None); // no lang

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_family,
            ComputedValues::initial().font_family,
            ":lang(ja) must not match when no element in the chain has a lang attribute"
        );
    }

    /// A closer ancestor's `lang` must win over a farther one — regression
    /// pin for `effective_language`'s "own first, then nearest ancestor"
    /// (not "any ancestor") walk order.
    #[test]
    fn lang_prefers_nearest_ancestor_lang_over_farther_one() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(en) { font-family: serif-en }");
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "ja");
        let section = doc.push_element(html, "section", None);
        doc.set_attr(section, "lang", "en");
        let p = doc.push_element(section, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "serif-en");
    }

    /// reviewer:spec finding (2026-08-12): HTML LS §3.2.6.2's own-`lang`
    /// step is scoped to "an HTML element or an element in the SVG
    /// namespace" (quoted in full on `effective_language`'s doc) — a MathML
    /// element's own `lang` attribute must NOT be consulted, unlike SVG's
    /// (`own_html_or_svg_lang_attribute`'s 2-element allowlist). Concrete
    /// spec example from the finding: `<html lang="en"><math lang="ja">…`
    /// — `<math>`'s own `lang="ja"` is ignored, so its effective language
    /// falls through to the `<html>` ancestor's `"en"`.
    #[test]
    fn lang_on_mathml_namespace_element_is_ignored_falls_through_to_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":lang(en) { font-family: en-font } :lang(ja) { font-family: ja-font }",
        );
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "en");
        let math = doc.push_element_with_namespace(
            html,
            "math",
            "http://www.w3.org/1998/Math/MathML",
            &[("lang", "ja")],
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[math].font_family[0].to_string(),
            "en-font",
            "MathML element's own lang attribute must be ignored per HTML \
             LS §3.2.6.2 (HTML/SVG only), falling through to the <html \
             lang=\"en\"> ancestor — :lang(en) must match, :lang(ja) must not"
        );
    }

    /// reviewer:spec backlog finding 3 (2026-08-12, low severity, optional):
    /// `:lang()`/`:dir()` on a non-rightmost compound — the left side of a
    /// combinator, matched via `match_from_ancestor` rather than the
    /// rightmost-element entry point in `match_complex_selector_list` — was
    /// reviewed by hand and judged structurally correct but untested.
    /// `:lang(ja) p` matches a `<p>` whose *grandparent* `<html>` (not its
    /// immediate parent `<body>`) carries `lang="ja"`, exercising both
    /// `match_from_ancestor`'s own `compound_matches` call (the `:lang(ja)`
    /// compound tested against the `<html>` ancestor candidate) and that
    /// call's `ancestors` slice (the further-out ancestors above `<html>` —
    /// here empty, but exercised as a real argument rather than `&[]`).
    #[test]
    fn lang_pseudo_class_matches_on_non_rightmost_compound_via_descendant_combinator() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) p { font-family: serif-ja }");
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "ja");
        let body = doc.push_element(html, "body", None);
        let p = doc.push_element(body, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_family[0].to_string(),
            "serif-ja",
            ":lang(ja) p must match <p> whose ancestor <html> (not <p> \
             itself) satisfies :lang(ja), via the ancestor-compound path \
             (match_from_ancestor) rather than the rightmost-element path"
        );
    }

    /// RFC 4647 §3.3.2 extended filtering: `:lang(en)` must match a more
    /// specific tagged element (`lang="en-US"`) — subtag prefix matching,
    /// not exact string equality.
    #[test]
    fn lang_range_matches_more_specific_tag_via_extended_filtering() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(en) { font-family: serif-en }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "lang", "en-US");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "serif-en");
    }

    /// `:dir(ltr)` matches an element with an explicit `dir="ltr"` attribute.
    #[test]
    fn dir_matches_explicit_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "dir", "ltr");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
    }

    /// `:dir(rtl)` matches an element with an explicit `dir="rtl"` attribute,
    /// and — the actual differentiator from `[dir=rtl]` this task's bd
    /// description points at — a descendant with no `dir` of its own
    /// inherits that directionality (`resolve_directionality`'s ancestor
    /// walk).
    #[test]
    fn dir_matches_explicit_rtl_and_inherits_to_descendant_without_own_dir() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(rtl) { font-family: rtl-font }");
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None); // no dir of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[article].font_family[0].to_string(),
            "rtl-font",
            ":dir(rtl) must match the element with the explicit attribute"
        );
        assert_eq!(
            r.computed[span].font_family[0].to_string(),
            "rtl-font",
            ":dir(rtl) must match a descendant with no dir attribute of its \
             own, inheriting from its dir=\"rtl\" ancestor"
        );
    }

    /// Default directionality (no `dir` attribute anywhere in the chain) is
    /// `ltr` — HTML LS "parent directionality" §3.2.6.4's root fallback
    /// (`resolve_directionality`'s doc). `:dir(rtl)` must not match; `:dir(ltr)`
    /// must.
    #[test]
    fn dir_defaults_to_ltr_when_no_dir_attribute_anywhere() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { font-family: rtl-font } :dir(ltr) { font-family: ltr-font }",
        );
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
    }

    /// RFC 4647 §3.3.2 extended filtering, direct pin (bypassing the cascade
    /// pipeline) of the examples the RFC itself gives for range `de-*-DE` /
    /// its synonym `de-DE` — [`language_range_matches`]'s doc quotes the
    /// algorithm this exercises.
    #[test]
    fn language_range_matches_rfc4647_de_de_examples() {
        // Matches (RFC 4647 §3.3.2, "matches all of the following tags"):
        for tag in [
            "de-DE",
            "de-de",
            "de-Latn-DE",
            "de-Latf-DE",
            "de-DE-x-goethe",
            "de-Latn-DE-1996",
            "de-Deva-DE",
        ] {
            assert!(
                language_range_matches("de-*-DE", tag),
                "de-*-DE must match {tag}"
            );
            assert!(
                language_range_matches("de-DE", tag),
                "de-DE (synonym) must match {tag}"
            );
        }
        // Does not match (RFC 4647 §3.3.2, "does not match any of the
        // following tags"):
        assert!(
            !language_range_matches("de-*-DE", "de"),
            "missing 'DE' subtag entirely"
        );
        assert!(
            !language_range_matches("de-*-DE", "de-x-DE"),
            "singleton 'x' occurs before 'DE', blocking the skip"
        );
        assert!(
            !language_range_matches("de-*-DE", "de-Deva"),
            "'Deva' present but 'DE' subtag never appears"
        );
    }

    #[test]
    fn language_range_matches_is_ascii_case_insensitive() {
        assert!(language_range_matches("EN", "en-us"));
        assert!(language_range_matches("en", "EN-US"));
    }

    /// `dir="auto"` folds into the same "keep walking up" bucket as a
    /// missing/invalid `dir` attribute — the documented scope cut on
    /// [`resolve_directionality`] (deferred Unicode-bidi content sniffing).
    #[test]
    fn dir_auto_falls_through_to_ancestor_directionality_not_default_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(rtl) { font-family: rtl-font }");
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let bdi_like = doc.push_element(article, "span", None);
        doc.set_attr(bdi_like, "dir", "auto"); // deferred, not resolved via bidi

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[bdi_like].font_family[0].to_string(),
            "rtl-font",
            "dir=\"auto\" must fall through to the ancestor's directionality \
             under this task's documented scope cut, not block inheritance"
        );
    }

    /// HTML LS §3.2.6.4 (quoted on `own_explicit_direction`'s doc): "the
    /// `dir` attribute is only defined for HTML elements \[...\] elements
    /// from other namespaces always end up using the parent
    /// directionality." A `dir="rtl"` attribute on a foreign-namespace
    /// (SVG) element must therefore be ignored, falling through to the
    /// ancestor walk — same shape as
    /// `resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive`'s
    /// SVG regression above.
    #[test]
    fn dir_attribute_on_foreign_namespace_element_is_ignored_falls_through_to_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
        let html = doc.push_element(0, "html", None); // no dir -> default ltr
        let svg = doc.push_element_with_namespace(
            html,
            "svg",
            "http://www.w3.org/2000/svg",
            &[("dir", "rtl")],
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[svg].font_family[0].to_string(),
            "ltr-font",
            "dir on a foreign-namespace element must be ignored, not treated \
             as an explicit directionality"
        );
    }

    #[test]
    fn language_range_matches_wildcard_matches_any_tagged_language() {
        // The CSS-spec-level "wildcard doesn't match untagged" rule
        // (`effective_language` doc) is enforced by `lang_pseudo_matches`
        // returning `false` on `None` before this function is ever called —
        // this function itself just needs to accept any non-empty tag for a
        // bare `*` range (RFC 4647 §3.3.2 step 2's wildcard-subtag clause).
        assert!(language_range_matches("*", "ja"));
        assert!(language_range_matches("*", "en-US"));
        assert!(language_range_matches("*", "und"));
    }

    // --- structural pseudo-classes (bd raikiri-spike-flln.5) ---
    //
    // `:root` (CSS Selectors L4 §13.1
    // <https://www.w3.org/TR/selectors-4/#the-root-pseudo>), `:empty`
    // (§13.2 <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>),
    // `:first-child`/`:last-child`/`:only-child`/`:nth-child()`/
    // `:nth-last-child()` (§13.3 area) and `:first-of-type`/`:last-of-type`/
    // `:only-of-type`/`:nth-of-type()`/`:nth-last-of-type()` (§13.4 area).
    // See `matches_empty`/`sibling_position`/`matches_nth` docs for the
    // full spec-provenance notes (L4's own TR anchors truncated on
    // WebFetch; fell back to Selectors Level 3, verbatim-quoted there).

    #[test]
    fn root_pseudo_class_matches_the_document_root_element_only() {
        // `background-color`, not `color`: `color` is inherited (CSS
        // Cascading L4 §5.2) — even a correct implementation that matched
        // `:root` on `<html>` alone would show a red `body.color` through
        // ordinary inheritance, making that a false-negative test for the
        // "must not also match a descendant" half (same pitfall
        // `child_combinator_applies_declaration_to_direct_child_only`'s own
        // comment documents). `background-color` is not inherited (CSS
        // Backgrounds 3 §2.2), so a red `body` here can only mean `:root`
        // itself wrongly matched it.
        //
        // `RuleTree::empty()` + `add_stylesheet`, not the usual
        // `push_element(0, "style", None)` + `build_rule_tree` convention
        // (quality-lens finding): that convention parks `<style>` itself as
        // a direct child of the Document node — i.e. an element sibling of
        // `<html>` that *also* has `parent_id.is_none()` and would *also*
        // match `:root`. Since this test never asserted anything about
        // `<style>`'s own computed value, that convention only proved "an
        // element with no element parent matches" (true of `html` here by
        // coincidence of push order), not "`:root` matches the root
        // element and no other top-level node" — the actual claim this
        // test's name makes. `RuleTree::empty()` avoids adding any such
        // ambiguous second candidate.
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", None);
        let body = doc.push_element(html, "body", None);

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(":root { background-color: red }", Origin::Author);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[html].background_color, RED,
            ":root must match the root element"
        );
        assert_eq!(
            r.computed[body].background_color,
            ComputedValues::initial().background_color,
            ":root must not match a non-root descendant"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_element_with_no_children() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match a childless element"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_element_with_zero_length_text_child() {
        // Spec text (`matches_empty` doc, verbatim): "...content nodes...
        // whose data has a non-zero length must be considered as affecting
        // emptiness" — a zero-length text node (`data.len() == 0`) does
        // NOT meet "non-zero length" and so must not disqualify `:empty`,
        // regardless of the L3/L4 whitespace-handling difference (quality
        // lens finding: this branch of `matches_empty`'s `Text` arm was
        // previously untested).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, "");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match an element with a zero-length text child"
        );
    }

    #[test]
    fn empty_pseudo_class_does_not_match_element_with_an_element_child() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_element(p, "span", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            ":empty must not match an element with an element child"
        );
    }

    #[test]
    fn empty_pseudo_class_does_not_match_element_with_a_text_child() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, "hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            ":empty must not match an element with a non-empty text child"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_whitespace_only_text_child() {
        // Acceptance-pinning test for the reviewer:spec correction
        // (`matches_empty` doc's "Spec provenance and correction" note):
        // CSS Selectors L4 *deliberately changed* `:empty` from L3 so that
        // whitespace-only content — "given white space is largely
        // collapsible in HTML and is therefore used for source code
        // formatting" (L4 changelog note, verbatim) — no longer
        // disqualifies. The L4 spec's own worked example lists `<p> </p>`
        // among what `p:empty` matches, verbatim. This test used to assert
        // the opposite (the pre-correction L3-only reading); inverted, not
        // just renamed, when the bug was fixed.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, " ");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match an element with a document-white-space-only text child (CSS Selectors L4)"
        );
    }

    #[test]
    fn empty_pseudo_class_does_not_match_nbsp_only_text_child() {
        // No-break space (U+00A0) is explicitly NOT a "document white
        // space character" (CSS Text 4, `is_document_white_space` doc) —
        // the L4 spec's own worked example lists `<div>&nbsp;</div>`
        // among what `div:empty` does *not* match, verbatim. Distinguishes
        // this from the (now-passing) plain-space case above: `:empty`'s
        // L4 whitespace carve-out is narrower than "any Unicode
        // whitespace".
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, "\u{00A0}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            ":empty must not match an element with an NBSP-only text child"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_element_with_only_a_comment_child() {
        // "comments... must not affect whether an element is considered
        // empty" (CSS Selectors L4 §13.2 `#the-empty-pseudo`, verbatim,
        // unchanged from L3) — a comment-only element still matches
        // `:empty`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_comment(p, " note ");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match an element whose only child is a comment"
        );
    }

    #[test]
    fn first_child_last_child_only_child_ignore_text_node_siblings() {
        // CSS Selectors L3 §6.6 preamble (verbatim, `sibling_position`
        // doc): "Standalone text and other non-element nodes are not
        // counted when calculating the position of an element in its list
        // of siblings" — a text node between two <li> must not shift
        // indices.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "li:first-child { color: red } \
             li:last-child { background-color: red }",
        );
        let ul = doc.push_element(0, "ul", None);
        let first = doc.push_element(ul, "li", None);
        doc.push_text(ul, "\n  "); // whitespace between <li> siblings
        let last = doc.push_element(ul, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[first].color, RED,
            ":first-child must match despite an intervening text node"
        );
        assert_eq!(
            r.computed[last].background_color, RED,
            ":last-child must match despite an intervening text node"
        );
        assert_eq!(
            r.computed[first].background_color,
            ComputedValues::initial().background_color,
            "the first <li> must not also match :last-child"
        );
    }

    #[test]
    fn only_child_matches_the_sole_element_child_and_nothing_else() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "li:only-child { color: red }");
        let solo_wrap = doc.push_element(0, "ul", None);
        let solo = doc.push_element(solo_wrap, "li", None);
        let pair_wrap = doc.push_element(0, "ul", None);
        let pair_a = doc.push_element(pair_wrap, "li", None);
        let pair_b = doc.push_element(pair_wrap, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[solo].color, RED,
            ":only-child must match a sole <li>"
        );
        assert_eq!(
            r.computed[pair_a].color,
            ComputedValues::initial().color,
            ":only-child must not match when a sibling <li> exists"
        );
        assert_eq!(
            r.computed[pair_b].color,
            ComputedValues::initial().color,
            ":only-child must not match when a sibling <li> exists"
        );
    }

    #[test]
    fn nth_child_zebra_striping_acceptance() {
        // Acceptance (bd raikiri-spike-flln.5): `:nth-child(2n+1)` zebra
        // striping. Rows 1/3/5 (1-based) get the declaration, 2/4 don't.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "tr:nth-child(2n+1) { background-color: red }");
        let table = doc.push_element(0, "table", None);
        let rows: Vec<usize> = (0..5)
            .map(|_| doc.push_element(table, "tr", None))
            .collect();

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        for (i, &row) in rows.iter().enumerate() {
            let one_based = i + 1;
            let expect_red = one_based % 2 == 1;
            assert_eq!(
                r.computed[row].background_color,
                if expect_red {
                    RED
                } else {
                    ComputedValues::initial().background_color
                },
                "row {one_based} (0-based index {i}): nth-child(2n+1) zebra stripe mismatch"
            );
        }
    }

    #[test]
    fn nth_child_negative_b_and_explicit_index_forms() {
        // `:nth-child(3)` (a=0) and `:nth-child(-n+2)` (first two only) —
        // exercises `AnPlusB` beyond the simple odd/even case.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "li:nth-child(3) { color: red } li:nth-child(-n+2) { background-color: red }",
        );
        let ul = doc.push_element(0, "ul", None);
        let items: Vec<usize> = (0..4).map(|_| doc.push_element(ul, "li", None)).collect();

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[items[0]].background_color, RED,
            "index 1 in -n+2"
        );
        assert_eq!(
            r.computed[items[1]].background_color, RED,
            "index 2 in -n+2"
        );
        assert_eq!(
            r.computed[items[2]].background_color,
            ComputedValues::initial().background_color,
            "index 3 not in -n+2"
        );
        assert_eq!(
            r.computed[items[2]].color, RED,
            "index 3 matches nth-child(3)"
        );
        assert_eq!(
            r.computed[items[0]].color,
            ComputedValues::initial().color,
            "index 1 does not match nth-child(3)"
        );
    }

    #[test]
    fn nth_last_child_counts_from_the_end() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "li:nth-last-child(1) { color: red }");
        let ul = doc.push_element(0, "ul", None);
        let items: Vec<usize> = (0..3).map(|_| doc.push_element(ul, "li", None)).collect();

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[items[2]].color, RED,
            ":nth-last-child(1) must match the last element sibling"
        );
        assert_eq!(
            r.computed[items[0]].color,
            ComputedValues::initial().color,
            ":nth-last-child(1) must not match the first element sibling"
        );
    }

    #[test]
    fn first_of_type_last_of_type_only_of_type_are_restricted_to_matching_tag() {
        // Mixed-tag sibling list: <h2><p><p><h2> — the -of-type family must
        // count only same-tag siblings (CSS Selectors L3 §6.6, "an+b-1
        // siblings with the same expanded element name"), unlike plain
        // :first-child/:last-child/:only-child.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "h2:first-of-type { color: red } \
             h2:last-of-type { background-color: red } \
             p:only-of-type { border-top-style: solid }",
        );
        let section = doc.push_element(0, "section", None);
        let h2_first = doc.push_element(section, "h2", None);
        let p = doc.push_element(section, "p", None);
        let h2_last = doc.push_element(section, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h2_first].color, RED,
            "h2:first-of-type must match the first <h2> even though a <p> is its actual first-child"
        );
        assert_eq!(
            r.computed[h2_last].background_color, RED,
            "h2:last-of-type must match the second <h2>"
        );
        assert_eq!(
            r.computed[h2_first].background_color,
            ComputedValues::initial().background_color,
            "the first <h2> must not also match :last-of-type"
        );
        assert_eq!(
            r.computed[p].border.top.style,
            BorderStyle::Solid,
            "p:only-of-type must match the sole <p> despite <h2> siblings"
        );
    }

    #[test]
    fn nth_of_type_counts_only_same_tag_siblings() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:nth-of-type(2) { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.push_element(div, "h2", None);
        let p1 = doc.push_element(div, "p", None);
        doc.push_element(div, "h2", None);
        let p2 = doc.push_element(div, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p2].color, RED,
            "p:nth-of-type(2) must match the 2nd <p>, ignoring interleaved <h2> siblings"
        );
        assert_eq!(
            r.computed[p1].color,
            ComputedValues::initial().color,
            "p:nth-of-type(2) must not match the 1st <p>"
        );
    }

    #[test]
    fn root_element_matches_first_child_last_child_only_child_and_nth_child_1() {
        // The root element has no *element* parent, but per CSS Selectors
        // L3's "an+b-1 siblings before/after it" framing (`matches_nth`
        // doc) it still has a (trivial, size-1) sibling list — itself
        // alone under the Document node.
        //
        // Deliberately does NOT use the usual `push_element(0, "style",
        // None)` + `build_rule_tree` convention: that convention parks the
        // `<style>` element itself as a direct child of the Document node
        // (id 0) — i.e. as an *element sibling of the root element being
        // tested here*, which would make `<style>` the real first element
        // child and `<html>` the second, defeating the point of this test.
        // `RuleTree::empty()` + `add_stylesheet` supplies the CSS without
        // adding any DOM node at all.
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", None);

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "html:first-child { color: red } \
             html:last-child { background-color: red } \
             html:only-child { border-top-style: solid } \
             html:nth-child(1) { border-bottom-style: solid }",
            Origin::Author,
        );
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[html].color, RED,
            "root element must match :first-child"
        );
        assert_eq!(
            r.computed[html].background_color, RED,
            "root element must match :last-child"
        );
        assert_eq!(
            r.computed[html].border.top.style,
            BorderStyle::Solid,
            "root element must match :only-child"
        );
        assert_eq!(
            r.computed[html].border.bottom.style,
            BorderStyle::Solid,
            "root element must match :nth-child(1)"
        );
    }

    #[test]
    fn root_element_does_not_match_nth_child_2() {
        // Negative half of the previous test (WPT reference:
        // `css/selectors/child-indexed-no-parent.html`, per CSS Selectors
        // L3's "an+b-1 siblings before it" framing this crate follows):
        // the root element's sibling list under `dom.root_id()` has size
        // 1 (itself alone), so no `:nth-child(N)`/`:nth-last-child(N)` for
        // `N >= 2` can ever match it. `:root:nth-last-child(2)` is the
        // canonical form of this check.
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", None);

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(":root:nth-last-child(2) { color: red }", Origin::Author);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[html].color,
            ComputedValues::initial().color,
            ":root:nth-last-child(2) must not match — the root element has no siblings at all"
        );
    }

    #[test]
    fn section_gt_p_first_child_acceptance() {
        // Acceptance (bd raikiri-spike-flln.5, audit doc top gap example
        // verbatim): `.section > p:first-child { font-weight: bold }`.
        // Combines the child combinator (bd raikiri-spike-flln.2) with a
        // structural pseudo-class on the *rightmost* compound.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".section > p:first-child { font-weight: bold }");
        let section = doc.push_element(0, "div", None);
        doc.set_attr(section, "class", "section");
        let first_p = doc.push_element(section, "p", None);
        let second_p = doc.push_element(section, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[first_p].font_weight, 700.0,
            ".section > p:first-child must match the first <p>"
        );
        assert_eq!(
            r.computed[second_p].font_weight,
            ComputedValues::initial().font_weight,
            ".section > p:first-child must not match the second <p>"
        );
    }

    #[test]
    fn structural_pseudo_class_on_an_ancestor_compound_uses_that_ancestors_own_parent() {
        // `body > div:only-child p` — the structural pseudo-class sits on
        // the *ancestor* compound (`div:only-child`), reached by crossing
        // the child combinator via `match_from_ancestor`, not on the
        // rightmost compound. This is the one test that would catch a
        // wrong `parent_id` slice at that recursion site (using `elem`'s
        // parent instead of the ancestor-being-matched's own parent).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "body > div:only-child p { color: red }");
        let body = doc.push_element(0, "body", None);
        let solo_div = doc.push_element(body, "div", None); // only element child of <body>
        let p_under_solo = doc.push_element(solo_div, "p", None);

        let other_body = doc.push_element(0, "body", None);
        let div_a = doc.push_element(other_body, "div", None);
        doc.push_element(other_body, "div", None); // makes div_a NOT an only-child
        let p_under_div_a = doc.push_element(div_a, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p_under_solo].color, RED,
            "body > div:only-child p must match when the <div> really is body's only child"
        );
        assert_eq!(
            r.computed[p_under_div_a].color,
            ComputedValues::initial().color,
            "body > div:only-child p must not match when the <div> has a sibling <div>"
        );
    }

    #[test]
    fn nth_child_of_extended_syntax_is_rejected_not_silently_widened() {
        // `:nth-child(An+B of S)` (CSS Selectors L4's extended
        // selector-list form) is out of scope — `RaikiriSelectorParser`
        // does not override `Parser::parse_nth_child_of` (default `false`,
        // `ruletree.rs`'s `is_supported_selector_list` doc). Per
        // `cssparser::Parser::parse_nested_block`'s own contract ("The
        // result is overridden to an `Err(..)` if the closure leaves some
        // input before that point", cssparser 0.37.0 `parser.rs`, direct
        // fetch) this must be a hard parse error — NOT silently parsed as
        // plain `:nth-child(An+B)` (which would silently match a superset
        // of what the author wrote).
        assert!(
            crate::parse_selector_list("p:nth-child(2n+1 of .foo)").is_err(),
            "the `of S` extended nth-child syntax must fail to parse, not silently widen"
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

    /// Same DOM shape as [`deep_chain_doc`], but the stylesheet's selector is
    /// a `depth`-compound **child**-combinator chain (`div > div > ... > div`)
    /// instead of a single compound (`div`). Child combinator is chosen over
    /// descendant here because `Combinator::Child` has exactly one candidate
    /// per level (`ancestors.split_last()`, no backtracking), so cost is
    /// O(`depth`) — this isolates the pure *stack-depth* question bd
    /// raikiri-spike-8r16 asks. A descendant-combinator version was tried
    /// first and rejected for an unrelated reason, not stack depth: see bd
    /// raikiri-spike-3p3h (`Combinator::Descendant`'s backtracking is
    /// separately exponential on uniformly-matching chains — discovered by
    /// that attempt).
    ///
    /// Before bd raikiri-spike-8r16's fix, this forced the
    /// `match_combinator_chain`/`match_from_element` mutual recursion
    /// (renamed from `match_from_ancestor`, bd raikiri-spike-flln.3) to a
    /// depth of `depth - 1` (one combinator per ancestor level), 2 native
    /// stack frames per level, independently of `collect_cascaded`'s own
    /// (already-iterative, job 199) DOM-DFS depth. Post-fix, this same
    /// `depth - 1`-long chain instead drives the explicit `Vec<Frame>` stack
    /// [`match_combinator_chain`]'s "Implementation" doc describes, with no
    /// native call-stack recursion involved at all.
    fn deep_child_combinator_chain_doc(depth: usize) -> (TestDoc, usize) {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
        }
        let style = doc.push_element(parent, "style", None);
        let selector = vec!["div"; depth].join(" > ");
        doc.push_text(style, &format!("{selector} {{ color: red }}"));
        (doc, parent)
    }

    /// Empirical answer for bd raikiri-spike-8r16: does a long, uniformly-
    /// matching child-combinator chain stack-overflow
    /// `match_combinator_chain`/`match_from_element`'s mutual recursion? Same
    /// small-fixed-stack technique as [`deep_nesting_small_stack_no_overflow`]
    /// (job 199 precedent) — deterministic, hardware/platform-independent.
    /// Unlike that test, `doc`/`tree` are built on the *main* (default-size)
    /// stack and only borrowed into the constrained-stack scoped thread —
    /// selector parsing (`build_rule_tree`, `selectors`/`cssparser` crate
    /// internals, outside this crate's cleanroom review surface) is
    /// deliberately excluded from the measured region, so a crash here can
    /// only be attributed to `cascade`'s own matching path.
    ///
    /// Confirmed empirically at depth 500 / 128 KiB stack against the
    /// mutually-recursive pre-fix implementation: the spawned thread
    /// aborted the whole process with `thread '<unknown>' has overflowed
    /// its stack` / SIGABRT, the same crash shape
    /// [`deep_nesting_small_stack_no_overflow`]'s doc describes for
    /// `collect_cascaded` pre-job-199 — i.e. this was a real, not
    /// hypothetical, overflow risk, matching the bd raikiri-spike-8r16
    /// concern. After converting the recursion to the explicit-stack form
    /// below, this test completes cleanly and returns the correct computed
    /// color.
    #[test]
    fn deep_child_combinator_chain_small_stack_no_overflow() {
        let (doc, deepest) = deep_child_combinator_chain_doc(500);
        let tree = build_rule_tree(&doc);
        let color = std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .stack_size(128 * 1024)
                .spawn_scoped(scope, || {
                    let result = cascade(&doc, &tree).expect("cascade Ok");
                    result.computed[deepest].color
                })
                .expect("spawn thread");
            handle.join().expect(
                "thread must not stack-overflow matching a long, successively-matching \
                 child-combinator chain",
            )
        });
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
        // rank 化により Origin::UserAgent の Normal は Origin::Author の
        // Normal より常に低い rank になる (`cascade_rank` doc に正確な値
        // あり) ので UA rule が Author を上書きすることはない (source_order
        // に関わらず rank が優先)。
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
        // Normal Author > Normal UA (`cascade_rank` doc has the exact values)
        let cv = cascade_with_ua("p { display: block }", "p { display: inline }", "p", None);
        assert_eq!(cv.display, DisplayValue::Inline);
    }

    #[test]
    fn important_ua_beats_important_author_display() {
        // Important UA > Important Author (!important 反転、`cascade_rank`
        // doc has the exact values)
        let cv = cascade_with_ua(
            "p { display: block !important }",
            "p { display: inline !important }",
            "p",
            None,
        );
        assert_eq!(cv.display, DisplayValue::Block);
    }

    // ── cascade_rank 4-tier ordering (3rd tier: bd raikiri-spike-wo36, CSS
    // Cascading L5 §6.5 "author presentational hint origin"; 4th tier: bd
    // raikiri-spike-pdta, CSS Cascading L4 §6.2 "user origin") ──

    #[test]
    fn cascade_rank_orders_ua_user_hint_author_normal_then_reverses_for_important() {
        // Direct unit exercise of `cascade_rank`'s full 8-arm match
        // (`cascade_rank`'s doc has the full re-derivation and rank table).
        // Pinned via a chain of relative-order assertions rather than exact
        // `assert_eq!` values: the chain below establishes a complete total
        // order over all 8 values (each value related to its neighbor, no
        // gaps) — it does *not* prove the exact literal values 0-7 (e.g.
        // ranks 10,11,12,14,15,16,17,18 would satisfy the same chain), but
        // that's the right level of strength here, since none of
        // `cascade_rank`'s 3 call sites — `counter_style.rs`'s
        // `insert_with_origin`, `page.rs`'s `cascade_page`, this file's
        // `beats` — switch on the literal `u8`, only compare/order it.
        //
        // The Normal-tier ordering (UA < User < hint < Author) combines two
        // spec-verbatim facts: CSS Cascading L4 §6.1's origin list gives
        // UA < User < Author directly, and CSS Cascading L5 §6.5's verbatim
        // text places the hint strictly between User and Author.
        //
        // The Important-tier ordering (Author < hint < User < UA) also
        // combines two facts, but only one is spec-verbatim: §6.1 directly
        // gives Author < User < UA for the important tier — exactly the
        // reverse of the normal-tier UA < User < Author order, read
        // straight off §6.1's list, not an analogy. The hint's position in
        // that reversal is *not* spec-verbatim: §6.5 never defines an
        // important presentational hint (host languages only ever emit
        // normal-tier hints), so this half pins `cascade_rank`'s own
        // minimal, non-arbitrary extension of that same reversal mechanism
        // to the hint (origin independence + CSS Cascading L4 §6.3's
        // importance reversal, <https://www.w3.org/TR/css-cascade-4/#importance>)
        // and its status as a total function, not an external requirement.
        // No production code path emits an `!important` presentational
        // hint (`push_img_dimension_hints` always pushes
        // `important = false`), and no production code path routes any
        // declaration to `Origin::User` yet (`Origin::User`'s doc has the
        // "no production producer" status, bd raikiri-spike-d7h3 tracks
        // adding one) — this test is the only place the 3
        // currently-production-unreached arms (`(User, false)`,
        // `(User, true)`, `(AuthorPresentationalHint, true)`) are
        // exercised.
        let normal_ua = cascade_rank(Origin::UserAgent, false);
        let normal_user = cascade_rank(Origin::User, false);
        let normal_hint = cascade_rank(Origin::AuthorPresentationalHint, false);
        let normal_author = cascade_rank(Origin::Author, false);
        let important_author = cascade_rank(Origin::Author, true);
        let important_hint = cascade_rank(Origin::AuthorPresentationalHint, true);
        let important_user = cascade_rank(Origin::User, true);
        let important_ua = cascade_rank(Origin::UserAgent, true);

        // Spec-verbatim (§6.1): Normal UA < Normal User < Normal Author.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_ua < normal_user,
            "normal UA must lose to normal user"
        );
        // Spec-verbatim (§6.5): the hint sits strictly between Normal User
        // and Normal Author. `normal_user < normal_author` follows
        // transitively from this assert and the next one, so it isn't
        // pinned separately.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_user < normal_hint,
            "normal user must lose to the hint"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_hint < normal_author,
            "normal hint must lose to a real author declaration"
        );
        // Spec-verbatim (§6.1/§6.3): any important declaration beats any
        // normal declaration.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_author < important_author,
            "any important declaration must beat any normal declaration"
        );
        // Spec-verbatim (§6.1): Important Author < Important User < Important UA.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            important_user < important_ua,
            "important user must lose to important UA"
        );
        // Derived (not spec-verbatim, see comment above): symmetry of
        // origin independence under importance reversal places the hint
        // strictly between Important Author and Important User, mirroring
        // its Normal-tier position between Normal User and Normal Author.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            important_author < important_hint,
            "derived symmetry: important hint ranks above important author"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            important_hint < important_user,
            "derived symmetry: important user ranks above important hint"
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
        // Cascade-origin pin (bd raikiri-spike-wo36): presentational hint は
        // `Origin::AuthorPresentationalHint`、inline style は `Origin::Author`
        // (`push_img_dimension_hints` doc の "Cascade origin" 節) — 別 origin
        // tier なので `cascade_rank` の rank 差だけで無条件に決着し、
        // inline style の specificity (`INLINE_SPECIFICITY` = `1 << 30`) を
        // 参照するまでもなく勝つ。
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
        // Origin-rank pin (bd raikiri-spike-wo36, `push_img_dimension_hints`
        // doc's "Cascade origin" section): the hint is
        // `Origin::AuthorPresentationalHint` (rank below `Origin::Author`
        // per `cascade_rank`), while this `* { width: 30px }` rule is a
        // real `Origin::Author` rule with zero specificity (universal
        // selector). Before bd raikiri-spike-wo36 both sides shared
        // `Origin::Author` and this exact zero-specificity/zero-source-order
        // case only resolved via `collect_cascaded`'s push-order (hint
        // pushed first, so the later-scanned real rule won the `beats` tie).
        // Now the rank difference alone decides it, independent of
        // specificity or push order — this test still pins "real author
        // rule wins regardless of specificity", just via a different
        // mechanism.
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
            "real author rule must win over the hint via origin rank, regardless of specificity"
        );
    }

    #[test]
    fn img_tag_name_match_is_ascii_case_insensitive() {
        // `push_img_dimension_hints` 自身の `elem.tag_name().eq_ignore_ascii_case`
        // 判定を確認 — `compound_matches` の `Component::LocalName` 判定
        // (bd raikiri-spike-flln.1、旧名 `match_by_tag`; bd raikiri-spike-flln.2
        // で `match_simple_selectors` → `compound_matches`/
        // `match_complex_selector_list` に分割 rename) と同じ寛容さの、独立
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

    #[test]
    fn apply_value_direct_overflow_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above (bd raikiri-spike-cmd3): `apply_value`'s
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
