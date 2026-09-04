//! CSS cascade + inheritance walk。
//!
//! 2 phase:
//! 1. per-node "cascaded values" 決定 — matching rule + inline style の候補集合から
//!    specificity + !important + source order で winner を選択
//! 2. inheritance walk — top-down DFS で親の computed value を継承 + 自 node の
//!    cascaded value で override
//!
//! # inheritance walk 内部の 4 段階
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
use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use cssparser::{Parser, ParserInput, Token};
use selectors::attr::{CaseSensitivity, ParsedCaseSensitivity};
use selectors::parser::{
    Combinator, NthOfSelectorData, NthSelectorData, Selector, SelectorIter, SelectorList,
};
use smol_str::SmolStr;

use crate::computed::{
    ComputedValues, CustomPropertyEnvironment, RunningTemplate, empty_custom_properties,
};
use crate::error::CascadeError;
use crate::property::{
    CustomProperty, DeferredValue, FontWeightValue, Length, LengthOrAuto,
    MAX_DEFERRED_VALUE_NESTING_DEPTH, MAX_SUBSTITUTED_VALUE_BYTES, PositionValue, PropertyValue,
    RelativeFontSize, is_custom_property_name, parse_value, resolve_text_align_match_parent,
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
/// 現時点では `computed` のみ populate。将来の GCPM (paged media generated
/// content) static-side 実装では、per-node ComputedValues 内で content /
/// string_set / running_templates を保持する canonical taxonomy に落ち着く見込みで、
/// CascadeResult-level の `gcpm_directives` / `running_templates` は下流
/// (raikiri-dom) で per-document に concatenate される責務に移る。
/// `#[non_exhaustive]` は将来 field 追加のために維持。
#[derive(Debug)]
#[non_exhaustive]
pub struct CascadeResult {
    /// Per-node computed values (NodeId.0 as usize で index)。
    /// Element / Text / Document 全 kind に populate、範囲外は panic (caller 責任)。
    pub computed: Vec<ComputedValues>,
}

/// DOM + RuleTree から per-node ComputedValues を produce。
///
/// 現時点では常に `Ok` を返す (invalid CSS は既に build_rule_tree 段で silently
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
/// let result = cascade(dom, &rule_tree).expect("現時点では常に Ok");
/// let root_style: &ComputedValues = &result.computed[0];
/// # }
/// ```
pub fn cascade<D: StyleDom>(dom: &D, rule_tree: &RuleTree) -> Result<CascadeResult, CascadeError> {
    let mut cascaded = CascadedArena::new();

    // Phase 1: per-node cascaded values を収集
    collect_cascaded(dom, dom.root_id(), rule_tree, &mut cascaded);

    // Phase 2: inheritance walk。
    //
    // computed を Dom::node_count() で pre-allocate する。resolve_inheritance の
    // DFS は root reachable な node のみを訪問するため、detached / unreachable
    // node (foster-parenting transient、strip 後の孤児 stub 等) には entry を
    // 作らない。しかし `computed.len() == document.node_count()` という contract
    // は arena 全体を要求する (`raikiri-dom::layout::preshape_text` /
    // `raikiri-paint::text::draw_text_node` が node_id で `computed[idx]` に
    // 直接 index する)。事前に initial() で埋めておき、DFS で visited slot を
    // 上書きする実装。
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
/// `tests::inline_specificity_exceeds_max_reachable_packed_specificity` を参照。
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
/// hint 専用の `cascade_rank` tier ([`Origin::AuthorPresentationalHint`]) を
/// 導入して以降、この tie は実際には発生しない — rank 差が specificity/source_order より先に
/// tuple compare で決着するため ([`push_img_dimension_hints`] doc の
/// "Cascade origin" 節参照)。`0` という値自体は「他候補と衝突しない値」を
/// 意図したものではなく、単に real stylesheet rule の source_order と同じ
/// 値域を使うという単純さのための選択。
const PRESENTATIONAL_HINT_SOURCE_ORDER: u32 = 0;

/// Margin-collapsing quirks zeroing declaration
/// ([`push_margin_collapsing_quirk_declarations`]) specificity. Pushed with
/// [`Origin::UserAgent`], so it never needs to out-rank a real
/// [`Origin::Author`] declaration on specificity — [`cascade_rank`] already
/// guarantees any `Origin::Author` declaration outranks any
/// `Origin::UserAgent` one regardless of specificity. This constant only
/// has to out-rank *other* `Origin::UserAgent` declarations for the same
/// property on the same node — most notably minimal.css's `blockquote,
/// figure, listing, p, plaintext, pre, xmp { margin-top: 1em;
/// margin-bottom: 1em; }` rule.
///
/// Reuses [`INLINE_SPECIFICITY`]'s value rather than re-deriving an
/// equivalent bound: that constant's own pinning test
/// (`inline_specificity_exceeds_max_reachable_packed_specificity`) already
/// proves it exceeds every specificity value reachable through the
/// `selectors` crate's packed representation — the exact same bound any
/// selector-based `Origin::UserAgent` rule (present or future) is subject
/// to as well.
const MARGIN_COLLAPSING_QUIRK_SPECIFICITY: Specificity = INLINE_SPECIFICITY;
/// [`MARGIN_COLLAPSING_QUIRK_SPECIFICITY`]'s companion source_order. `0` —
/// same reasoning as [`PRESENTATIONAL_HINT_SOURCE_ORDER`]: the
/// dedicated `cascade_rank` tier already decides every comparison that
/// matters (against other `Origin::UserAgent` declarations, `beats`'s
/// specificity comparison is what actually separates this from
/// minimal.css's rule, and no two of *this* function's own pushes ever
/// compete against each other for the same property on the same node), so
/// no value other than "the same low end of the range real stylesheet
/// source_order uses" is needed here.
const MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER: u32 = 0;

/// 1 candidate declaration = `(value, important, origin, specificity, source_order)`。
/// `collect_cascaded` が populate、`pick_winners` が rank 化して winner を選ぶ
/// (`Origin` を含む — clippy::type_complexity 回避のため alias 化)。
type CascadedDecl = (PropertyValue, bool, Origin, Specificity, u32);

/// A custom-property candidate. Unlike ordinary declarations, custom
/// properties are keyed by their case-sensitive name rather than by a fixed
/// `PropertyKey` slot.
type CustomCascadedDecl = (CustomProperty, bool, Origin, Specificity, u32);

type InheritanceStackEntry = (
    StyleNodeId,
    ComputedValues,
    Option<ResolveContext>,
    Arc<CustomPropertyEnvironment>,
);

/// [`collect_cascaded`] の出力 — 全 node 分の candidate を単一 flat `Vec` に
/// 積み、node ごとの部分区間を [`Range`] で引く。
///
/// # 何を置換したか
///
/// 旧実装は `HashMap<StyleNodeId, Vec<CascadedDecl>>` — per-node に `Vec` を
/// 1 本ずつ確保していた。n=1000 node の cascade で **collect_cascaded 単体
/// 4,030 allocs / 2,439,940 bytes** (実測値)。ただしこの 4,030 のうち
/// **1,020 allocs / 64,744 bytes は本 struct が触れていない
/// `dom.child_ids(id).collect()` 行**に由来していた (同じ doc でその行だけを
/// 単独実行して確認、当時は本 struct の対象外)。この残差は、
/// [`collect_cascaded`] / [`resolve_inheritance`] 双方の呼び出し箇所を「捨て
/// `Vec` へ `collect` して `rev()`」から「`stack` へ直接 `extend` してから
/// 追加分だけ in-place `reverse()`」に書き換えることで解消済み — 中間
/// allocation はもう存在しない (同じ形の第 3 の call site だった
/// `crates/raikiri-style/src/ruletree.rs` の `walk_style_elements` も同じ
/// 技法で解消済み、本 module の対象外)。per-node `Vec` の growth chain 自体が
/// 担っていたのは残り **3,010 allocs / 2,375,196 bytes** — push のたび
/// geometric に再確保するその growth chain が丸ごと allocation cost だった。
/// 単一 arena にすると growth chain は文書全体で 1 本になり (n=1000 で 23
/// allocs まで低下、-99.2%)、chain 長は `O(log 総 candidate 数)` に潰れる。
///
/// # なぜ struct で wrap するか (bare `(Vec<_>, HashMap<_, Range<usize>>)` にしないか)
///
/// [`pick_winners`] の `winner.idx` は「渡された **その** slice 内の位置」で
/// あり、**範囲外にならず静かに別 node の宣言を読む** 経路がある。arena 化で新たに生まれる同型の
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
    /// All custom-property candidates in document visit order.
    custom_decls: Vec<CustomCascadedDecl>,
    /// Per-node ranges into `custom_decls`.
    custom_ranges: HashMap<StyleNodeId, Range<usize>>,
}

impl CascadedArena {
    fn new() -> Self {
        Self {
            decls: Vec::new(),
            ranges: HashMap::new(),
            custom_decls: Vec::new(),
            custom_ranges: HashMap::new(),
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

    fn custom_candidates(&self, id: StyleNodeId) -> Option<&[CustomCascadedDecl]> {
        self.custom_ranges
            .get(&id)
            .map(|range| &self.custom_decls[range.clone()])
    }
}

fn push_cascaded_decl(
    out: &mut CascadedArena,
    value: PropertyValue,
    important: bool,
    origin: Origin,
    specificity: Specificity,
    source_order: u32,
) {
    match value {
        PropertyValue::CustomProperty(custom) => {
            out.custom_decls
                .push((custom, important, origin, specificity, source_order))
        }
        value => out
            .decls
            .push((value, important, origin, specificity, source_order)),
    }
}

/// [`pick_winners`] の scratch slot — 1 property key の暫定勝者。
///
/// [`idx`](Self::idx) が [`PropertyValue`] 本体ではなく **index** なのが要点:
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

/// Cascade origin + `!important` flag に基づく優先度 rank。
/// [`Origin::AuthorPresentationalHint`] と [`Origin::User`] の 2 origin は
/// 当初の UA/Author 2-origin 実装に後から追加されたものであり、特に
/// [`Origin::User`] の追加時には単純な番号ずらしではなく 4-tier 全体を
/// re-derive している (下記参照)。
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
/// 言葉遣いと上記引用群を突き合わせた this crate の解釈 (spec 上の直接の
/// 裏付けがない defensible な judgment call)。
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
/// # 4-tier 全体の rank 表 (re-derivation)
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
/// push する)。`(User, false)` / `(User, true)` の 2 arm は当初どの
/// production 呼び出し元からも到達しなかったが、consumer 提供
/// `extra_stylesheets` が [`Origin::User`] へ route されるようになったため、
/// 今は両方とも到達する
/// ([`Origin::User`] の doc 参照) — [`crate::page::cascade_page`] も同じ
/// [`Origin`] を経由するため、これらも `unreachable!()` にはせず total
/// function として値を返す (この判断自体は producer の有無に関わらず
/// 元々正しかった)。
///
/// `revert` keyword carve-out (上記 4 番目の引用: "it is considered part of
/// the author origin" — `revert-layer` は対象外) は本 crate に現状影響しない
/// — `revert`/`revert-layer` CSS-wide keyword 自体がまだ未実装
/// ([`crate::property`] の "CSS-wide keyword (canonical)" 節参照)。実装時に
/// この carve-out の special-case が必要になる。
///
/// `@page` cascade も同じ origin ordering を共有するため
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

/// `collect_cascaded` は DFS で node を訪れる。descendant/child combinator に
/// 未対応だった頃は「per-node の処理は他の node の状態に依存しないため
/// 訪問順は無関係」だった。descendant/child combinator の追加でこの前提は
/// **もう成り立たない** — 各 element の selector matching は本関数 local の
/// `ancestor_path`（「これまでに訪れた祖先 element の id 列」）を参照するため、
/// **祖先を子孫より先に処理する pre-order 訪問が正しさの前提**になった (祖先が
/// 先に積まれていなければ descendant/child の ancestor 参照が空振りする)。
/// overflow 回避のため explicit `Vec` stack で iterative に書く方針
/// 自体は変わらないが、stack の要素は素の `StyleNodeId`
/// ではなく `(StyleNodeId, usize)` — 後者は「この node を処理する直前に
/// ancestor path を truncate すべき長さ」。詳細は本関数の実装コメント参照。
///
/// # flat arena への書き込み
///
/// 1 node 分の candidate は `out.decls` に**連続して**積まれる —
/// stylesheet rule matching (rule/declaration の source order) → inline
/// style の順に push し、両方終わったところで `start..out.decls.len()` を
/// その node の区間として登録する。次の node の処理が始まるまで他の push が
/// 割り込まないことが「区間が連続」の根拠であり、
/// [`CascadedArena::candidates`] が返す slice の index が
/// [`pick_winners`]/[`apply_winners`] にとって**その node 自身の**
/// `candidates` 内 index であり続ける前提そのもの (global index space を
/// そのまま渡すと壊れる、という点に注意)。
fn collect_cascaded<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut CascadedArena,
) {
    // Document-wide constant — read once rather than
    // per (node, rule) pair inside the loop below.
    let quirks_mode = dom.quirks_mode();
    // Stack entries pair a node id with the `ancestor_path` length it should
    // be truncated to *before* that node is processed.
    // `stack` itself interleaves the pending work of
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
            // <template> 子孫 + 将来の inert subtree を統一 skip。
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
                let custom_start = out.custom_decls.len();
                // HTML presentational hints (later retagged to
                // `Origin::AuthorPresentationalHint`, distinct from plain
                // `Origin::Author`). This push is kept ahead of
                // stylesheet-rule matching / inline style below for
                // historical/document-order reasons, but it is no longer a
                // *correctness* requirement: since the hint has its own
                // `cascade_rank` tier (strictly between `UserAgent` and
                // `Author`, see `push_img_dimension_hints` doc's "Cascade
                // origin" section), rank alone decides against any real
                // Author-origin declaration regardless of specificity,
                // source_order, or push order — no tie can occur (that was
                // only possible earlier, when hint and real Author
                // declarations shared the same `Origin::Author` rank).
                // Re-verified after the 4th `Origin::User` tier was inserted:
                // the hint's `cascade_rank` value moved (see `cascade_rank`'s
                // rank table) but stayed strictly between `Origin::User` and
                // `Origin::Author` — never equal to the real `Author` rank in
                // either the Normal or the Important half of the table — so
                // this reasoning still holds unchanged; no test pins the push
                // order itself (nothing here is order-*dependent* left to
                // pin), but
                // `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
                // continues to pin the outcome this comment claims.
                push_img_dimension_hints(&elem, &mut out.decls);
                // HTML LS §15.3.9 margin-collapsing quirks (quirks-mode
                // margin-block zeroing) — `ancestor_path` here is still
                // `id`'s ancestor chain *without* `id` itself (that push
                // happens further below, after this element's candidates
                // are collected), so `ancestor_path.last()` is exactly
                // `id`'s real DOM parent. See
                // `push_margin_collapsing_quirk_declarations` doc for the
                // full rule set and design rationale.
                push_margin_collapsing_quirk_declarations(
                    dom,
                    id,
                    &elem,
                    &ancestor_path,
                    quirks_mode,
                    &mut out.decls,
                );
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
                            // post-parse mutation 経路を守れないため)。
                            // rationale は `crate::rule::expand_shorthand_into`
                            // doc に集約。
                            expand_shorthand_into(decl, |d| {
                                push_cascaded_decl(
                                    out,
                                    d.value,
                                    d.important,
                                    rule.origin,
                                    spec,
                                    rule.source_order,
                                );
                            });
                        }
                    }
                }
                // inline style
                if let Some(source) = elem.inline_style_source() {
                    let mut input = ParserInput::new(source);
                    let mut parser = Parser::new(&mut input);
                    for decl in parse_declaration_block(&mut parser) {
                        push_cascaded_decl(
                            out,
                            decl.value,
                            decl.important,
                            Origin::Author,
                            INLINE_SPECIFICITY,
                            INLINE_SOURCE_ORDER,
                        );
                    }
                }
                let end = out.decls.len();
                if end > start {
                    out.ranges.insert(id, start..end);
                }
                let custom_end = out.custom_decls.len();
                if custom_end > custom_start {
                    out.custom_ranges.insert(id, custom_start..custom_end);
                }
                // This element becomes an ancestor for its own children
                // (pushed just below with `ancestor_path.len()` as their
                // truncation depth).
                ancestor_path.push(id);
            }
            // stack は LIFO なので document order で push するため reverse。
            // `child_ids` イテレータを直接 `stack` へ `extend` し、今回追加した
            // 末尾スライスだけを in-place `reverse()` する — 都度捨てる中間
            // `Vec` を経由しない。`stack` 自体の
            // capacity growth は元の `for .. { stack.push(..) }` と同じ
            // amortized pattern のままで、ここで削れるのは「今回だけの捨て
            // Vec」1 本分のみ。
            //
            // なぜ document order (pre-order) を保つ**必要がある**か (descendant/
            // child combinator 対応の追加でここの結論が反転): 本関数冒頭のコメント
            // の通り、descendant/child combinator matching は
            // `ancestor_path` — DFS の訪問順そのもの — に依存する。子を
            // 親より先に訪れると `ancestor_path` にまだ親が積まれておらず、
            // 子の combinator matching が誤って不一致になる。旧
            // (combinator 非対応時代) の「訪問順は無関係、
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
/// 判定する最初の 1 手と、combinator 越しの祖先/兄弟候補判定
/// ([`match_combinator_chain`] / [`match_from_element`] — descendant/child
/// 越しの祖先判定と、NextSibling/LaterSibling 越しの兄弟判定は同じ関数に
/// 統合されている) の両方がこの関数を共有する — 元々の
/// `match_simple_selectors` 本体をそのまま抽出しただけで、per-component の
/// 判定ロジック自体に変更は無い。
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
///   厳密一致 ("limited-quirks" は DOM Standard上
///   "quirks mode" と別 dfn、fold の対象外)
/// - `Component::Class` — `elem.has_class()` / `elem.has_class_ascii_case_insensitive()`
///   (CSS Selectors L4 <https://www.w3.org/TR/selectors-4/#class-html>、
///   verbatim: "When matching against a document which is in quirks mode,
///   class names must be matched ASCII case-insensitively; class selectors
///   are otherwise case-sensitive")。ID と同じ `quirks_mode` 分岐。
///   both variants share the same HTML-spec ASCII
///   whitespace tokenisation — [`StyleElement::has_class`] の doc 参照
/// - `Component::AttributeInNoNamespaceExists` / `Component::AttributeInNoNamespace`
///   — `elem.attr()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#attribute-selectors>)。存在チェック
///   形態 (`[foo]`) の lookup key は element の namespace に応じて
///   `local_name` / `local_name_lower` を選ぶ (詳細は該当 match arm の
///   コメント)。値付き形態の case-sensitivity 解決は
///   [`resolve_case_sensitivity`] 参照
/// - `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
///   — [`language_range_matches`] /
///   [`resolve_directionality`] 経由、`dom` + `ancestors` (自身の祖先 chain)
///   を使って ancestor-inherited な effective language / directionality を
///   解決する。`PseudoClass::Hover` / `PseudoClass::Active` はこの arm 内で
///   引き続き `false` (dynamic pseudo-class の対応は本実装の scope 外のまま)。
/// - `Component::Root` (`:root`, CSS Selectors L4
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
/// - `Component::Empty` (`:empty`, CSS Selectors
///   L4 §13.2 <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — see
///   [`matches_empty`] doc for the verbatim spec text and its L4-vs-L3
///   whitespace-handling correction history.
/// - `Component::Nth(data)` (`:first-child`/`:last-child`/`:only-child`/
///   `:nth-child()`/`:nth-last-child()` and their `-of-type` counterparts,
///   CSS Selectors L4 §13.3/§13.4) — see
///   [`matches_nth`] doc for the sibling-position algorithm and its spec
///   citation. Reuses `ancestors.last().copied().unwrap_or_else(||
///   dom.root_id())` for its sibling-list parent — the exact same
///   root-fallback idiom [`match_combinator_chain`]'s `NextSibling`/
///   `LaterSibling` arms already established for
///   an unrelated reason (sibling lookup key, not a compound-match
///   target); both fall back for the same underlying reason ("the root
///   element's parent-in-tree is the Document node, not an `Element`, but
///   `StyleDom::child_ids` still works against it").
/// - `Component::NthOf(data)` (`:nth-child(An+B of S)` /
///   `:nth-last-child(An+B of S)`, CSS Selectors L4 §13.3.1/§13.3.2) — see
///   [`matches_nth_of`] for the filtered sibling-position algorithm. Each
///   direct element child is first matched against the stored selector-list
///   `S` using the same complex-selector matcher as a stylesheet selector;
///   only matching children contribute to the 1-based position.
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
                // 常に match / namespace は現時点では常に true 扱い
                true
            }
            // CSS Selectors L4 id-selectors / class-html (verbatim
            // quoted on the function doc
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
                    resolve_directionality(dom, elem, elem_id, ancestors) == *dir
                }
                // `:hover` / `:active` — dynamic pseudo-class は本実装の
                // scope 外のまま。`is_supported_selector_list` が rule tree 構築時点で
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
                // arms already use.
                let sibling_parent = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
                matches_nth(
                    dom,
                    sibling_parent,
                    elem_id,
                    elem.tag_name(),
                    data,
                    ancestors,
                    quirks_mode,
                )
            }
            Component::NthOf(data) => {
                let sibling_parent = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
                matches_nth_of(
                    dom,
                    sibling_parent,
                    elem_id,
                    elem.tag_name(),
                    data,
                    ancestors,
                    quirks_mode,
                )
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

/// `:empty` (CSS Selectors L4 §13.2
/// <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — whether
/// `elem_id` has no children that count toward emptiness.
///
/// # Spec provenance and correction
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
/// only element nodes and content nodes (such as \[\[DOM\]\] text nodes, and
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
/// here as a discrepancy to confirm or correct with a citation, since this
/// function currently follows the directly-verified primary source over
/// the relayed one where they disagree.
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

#[derive(Clone, Copy)]
struct SiblingMatchContext<'a> {
    selector_filter: Option<&'a [Selector<RaikiriSelectorImpl>]>,
    ancestors: &'a [StyleNodeId],
    quirks_mode: StyleQuirksMode,
}

/// 1-based sibling position of `elem_id` among `parent_id`'s **in-document
/// element** children, both from the start and from the end, plus the total
/// count of such siblings — shared arithmetic behind `Component::Nth` and
/// `Component::NthOf` matching ([`matches_nth`] / [`matches_nth_of`]).
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
/// always-true (`Component::DefaultNamespace(_) => true`, "常に
/// match / namespace は現時点では常に true 扱い") for the equivalent
/// selector-vs-element case, so restricting this sibling-vs-sibling
/// comparison to `tag_name` equality inherits that existing scope
/// simplification rather than introducing a new one. Plain `==` (not
/// `eq_ignore_ascii_case`, unlike the selector-vs-element `LocalName` arm)
/// — html5ever already normalises HTML tag names to lowercase before they
/// ever reach `StyleElement::tag_name`, and "expanded name" comparison for
/// non-HTML (SVG/MathML) content is case-sensitive per XML tag-name rules,
/// so exact comparison is correct for both.
///
/// When `selector_filter` is present, a child contributes only if it matches
/// at least one selector in that list. The candidate is evaluated with the
/// same `ancestors` and `quirks_mode` as the element being matched, because
/// all direct siblings share that parent context.
fn sibling_position<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    of_type: bool,
    context: SiblingMatchContext<'_>,
) -> (i32, i32, i32) {
    let mut total = 0i32;
    let mut index_from_start = 0i32;
    for child_id in dom.child_ids(parent_id) {
        let Some(child_node) = dom.node(child_id) else {
            continue;
        };
        // `child_ids` is the raw arena view for the style DOM, so detached /
        // inert nodes can still occur in this iterator. Structural
        // pseudo-classes operate on the flat-tree sibling list, matching the
        // gate used by `collect_cascaded` and sibling combinators.
        if !child_node.is_in_document() {
            continue;
        }
        let Some(sibling) = child_node.as_element() else {
            continue;
        };
        if of_type && sibling.tag_name() != elem_tag {
            continue;
        }
        if let Some(selectors) = context.selector_filter
            && !selector_slice_matches(
                selectors,
                dom,
                &sibling,
                child_id,
                context.ancestors,
                context.quirks_mode,
            )
        {
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
/// except for the `An+B of S` extension handled by [`matches_nth_of`]):
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
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    let (from_start, from_end, total) = sibling_position(
        dom,
        parent_id,
        elem_id,
        elem_tag,
        data.ty.is_of_type(),
        SiblingMatchContext {
            selector_filter: None,
            ancestors,
            quirks_mode,
        },
    );
    matches_nth_position(from_start, from_end, total, data)
}

/// `Component::NthOf` matching for CSS Selectors L4's `:nth-child(An+B of S)`
/// and `:nth-last-child(An+B of S)` forms. The specification defines the
/// position among the inclusive siblings that match `S`; the selector-list
/// matcher therefore runs for every direct element child before the normal
/// `An+B` arithmetic is applied.
fn matches_nth_of<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    data: &NthOfSelectorData<RaikiriSelectorImpl>,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    let nth_data = data.nth_data();
    let (from_start, from_end, total) = sibling_position(
        dom,
        parent_id,
        elem_id,
        elem_tag,
        nth_data.ty.is_of_type(),
        SiblingMatchContext {
            selector_filter: Some(data.selectors()),
            ancestors,
            quirks_mode,
        },
    );
    matches_nth_position(from_start, from_end, total, nth_data)
}

fn matches_nth_position(
    from_start: i32,
    from_end: i32,
    total: i32,
    data: &NthSelectorData,
) -> bool {
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
/// # Combinator 対応
///
/// 当初は single-element (compound-only) matching のみで、combinator を
/// 含む selector は `ruletree.rs` `is_supported_selector_list` の gate で
/// rule tree に乗る前に drop されていた。その後 descendant (space, CSS
/// Selectors L4 <https://www.w3.org/TR/selectors-4/#descendant-combinators>)
/// と child (`>`, <https://www.w3.org/TR/selectors-4/#child-combinators>)
/// の 2 combinator を追加し、続けて adjacent sibling (`+`,
/// <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>) と
/// general sibling (`~`,
/// <https://www.w3.org/TR/selectors-4/#general-sibling-combinators>) を追加
/// した (4 combinator 全対応、詳細は [`match_combinator_chain`] doc)。complex
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
/// (同関数の doc 参照)。`elem_id` は `elem` 自身の id — sibling combinator
/// が「`elem` の親の子リストの中で `elem` より前にいる
/// のは誰か」を [`StyleDom::child_ids`] から直接求める際の探索終端として
/// 導入され ([`match_combinator_chain`] の `NextSibling`/`LaterSibling` arm
/// 参照)、その後 [`compound_matches`] 自身にも渡すよう
/// 拡張された — `:root`/`:empty`/`:nth-child()` 等の構造的 pseudo-class が
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
        if selector_matches(dom, selector, elem, elem_id, ancestors, quirks_mode) {
            let spec = specificity_of(selector);
            best = Some(match best {
                Some(prev) => prev.max(spec),
                None => spec,
            });
        }
    }
    best
}

fn selector_slice_matches<D: StyleDom, E: StyleElement>(
    selectors: &[Selector<RaikiriSelectorImpl>],
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    selectors
        .iter()
        .any(|selector| selector_matches(dom, selector, elem, elem_id, ancestors, quirks_mode))
}

fn selector_matches<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    let mut iter = selector.iter();
    compound_matches(dom, &mut iter, elem, elem_id, ancestors, quirks_mode)
        && match iter.next_sequence() {
            None => true,
            Some(combinator) => {
                match_combinator_chain(dom, combinator, elem_id, ancestors, iter, quirks_mode)
            }
        }
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
///   **この再試行は load-bearing — 省略すると壊れる** (訂正: 以前ここには
///   「祖先チェーンは分岐の無い単一の直線なので retry は
///   冗長」という誤った一般化があった。独立したレビューが複数、
///   同型の反例を構築して指摘 — 以下は
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
///   「事情が変わる」というのは
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
/// - [`Combinator::NextSibling`] (CSS Selectors L4
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
/// - [`Combinator::LaterSibling`] (CSS Selectors L4
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
/// # 親の解決: `ancestors.last()` の空スライス fallback
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
/// error にする (`a::before` を直接 parse させて実地確認済み) ため、この
/// crate 内で生成された `SelectorList` から到達することは無い。
/// [`compound_matches`] の `_ => false` safety net と
/// 同じ姿勢で、ここでも到達したら match fail 扱いにする。
///
/// # Spec provenance note
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
/// # Implementation: explicit `Vec` stack, not native recursion
///
/// Prior to this fix, this function and [`match_from_element`]
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
///
/// # Memoization: bounding backtracking to polynomial time
///
/// The explicit-stack rewrite above removes the native-stack-overflow risk,
/// but on its own does nothing about a second, independent problem: the
/// backtracking itself. [`Combinator::Descendant`]'s retry (this doc's
/// "load-bearing" note above) tries every remaining ancestor as a candidate,
/// and — when a candidate's compound matches but everything further left
/// ultimately fails to complete the match — falls back to the next
/// (farther) candidate. When every remaining ancestor's compound matches
/// (e.g. a `div`-only complex selector against a chain of `<div>`s) and the
/// selector is ultimately unsatisfiable (typically: it needs more ancestor
/// "slots" than the chain actually has, at that point in the search), every
/// combination of candidates gets tried before the search can conclude
/// failure. Writing `S(k)` for the number of [`match_from_element`] calls
/// needed to *prove* failure with `k` ancestors available and always more
/// remaining compounds than ancestors (the unsatisfiable case) gives the
/// recurrence `S(0) = 1`, `S(k) = S(0) + S(1) + ... + S(k-1)` (one recursive
/// call per candidate, ancestors-remaining shrinking from `k-1` down to `0`
/// as the candidate gets farther from the target) — which solves to
/// `S(k) = 2^(k-1)` for `k >= 1`. A `div`-only complex selector with exactly
/// as many compounds as the ancestor chain is deep (the "exact fit" case —
/// every compound has a slot, no unsatisfiable point is ever reached) does
/// *not* hit this bound: it matches via the same greedy
/// nearest-candidate-first path this doc's [`Combinator::Descendant`] note
/// describes, and the retry loop is never actually exercised because the
/// very first candidate at every level already leads to a full match. The
/// same selector with **one extra compound** (so the chain is one ancestor
/// short of what the selector needs, the minimal unsatisfiable case) is the
/// one that hits `S(k) = 2^(k-1)` — confirmed empirically (this crate's
/// `TestDoc` mock, `match_complex_selector_list` timed directly) to match
/// this scaling. This is an untrusted-depth CPU-exhaustion vulnerability,
/// not merely a slow path: a sufficiently deep DOM chain
/// (attacker-controlled markup depth) matched against a same-shape selector
/// (attacker-controlled stylesheet) makes `S(k)` explode long before any
/// stack limit is reached — the *fixed* [`match_combinator_chain`] above
/// still performed 2^(k-1) [`match_from_element`] calls, just on the heap
/// instead of the native stack.
///
/// Content mismatch (not just a numeric shortfall) can trigger the exact
/// same blowup: e.g. a selector whose leftmost compound is `span` matched
/// against a chain where every ancestor is `div`, with the compound *count*
/// otherwise exactly matching the ancestor count. The `span` compound never
/// matches any candidate, so the search is just as unsatisfiable as the
/// "one compound too many" case above, and every combination of candidates
/// for the intervening `div` compounds still gets tried before the `span`
/// failure is reached each time — choosing an ancestor at position `i`
/// (instead of the nearest one) leaves `i - 1` ancestors for `m - 1`
/// remaining compounds, and unless `i` happens to be exactly the position
/// that keeps the count matching, this only defers the failure rather than
/// pruning it. A fix that only compares compound and ancestor *counts*
/// (rejecting whenever remaining compounds exceed remaining ancestors)
/// would close the first shape but not this one — an attacker can always
/// reach it with a one-token substitution in an otherwise-exact-fit
/// selector, e.g. swapping one `div` for a compound the DOM never has.
///
/// The fix memoizes **candidates already proven to fail** — same
/// motivation as memoized backtracking in text-pattern matching (the
/// classic fix for the analogous `a?a?a?...aaa...a` vs `aaa...a`
/// exponential-regex-backtracking shape), applied here to a fixed
/// ancestor/sibling chain instead of a string. The key insight: at the
/// point [`match_combinator_chain`]'s loop is about to try `candidate_id`
/// against the compound `frame` (the current top of the explicit stack)
/// introduces, "does `candidate_id` satisfy this compound *and* everything
/// further left" is a pure function of exactly two things —
///
/// - `candidate_id` itself (which pins down its exact position in the
///   fixed ancestor chain, and hence its own remaining-ancestors slice —
///   see the soundness argument below), and
/// - how many compounds are left to satisfy, i.e. how many frames are
///   currently on `stack` (`stack.len()`, called `depth` below — every
///   combinator, of all four kinds, pushes exactly one frame per level, so
///   this is a stable position marker for "which compound in the fixed
///   original chain are we about to test" regardless of how many
///   backtracking attempts came before).
///
/// So `(candidate_id, depth)` is used as the memo key. **Soundness
/// argument for why `depth` need not be paired with the ancestors slice
/// itself**: every `ancestors`/`candidate_ancestors` value that ever
/// appears anywhere in this search — whether produced by
/// [`Combinator::Child`]/[`Combinator::Descendant`]'s `split_last`-based
/// shrinking or inherited unchanged through a
/// [`Combinator::NextSibling`]/[`Combinator::LaterSibling`] jump (siblings
/// share a parent, hence share the same full ancestor chain — this doc's
/// "load-bearing" retry note establishes this for the shrinking case, and
/// [`match_from_element`]'s own doc establishes it for the sibling case) —
/// is always *some prefix* of the one fixed root-to-target
/// ancestor array this function's own `ancestors` parameter starts with
/// (shrinking only ever drops the array's own tail, and passing it through
/// unchanged is trivially still the same prefix). That original array has
/// no repeated node (it is a single straight ancestor chain in a tree, and
/// a tree has no cycles), so a specific node's position within it — and
/// hence the length of the "everything above this node" prefix — is fixed
/// once and for all by which node it is, independent of which backtracking
/// path reached it. So `candidate_id` alone already determines the
/// ancestors it will be evaluated against; there is no way for the same
/// `(candidate_id, depth)` pair to legitimately mean two different
/// sub-problems.
///
/// Recording: a candidate is memoized as failed either immediately (its
/// compound didn't match at all) or once the frame it was pushed into (for
/// its own further-left continuation) is fully exhausted without a match —
/// at that point `(candidate_id, depth)` is a *proven* dead end, valid for
/// every future backtracking path that might otherwise re-examine the same
/// candidate at the same position. The one exception is `depth == 1` (the
/// outermost frame, this function's own entry point): nothing ever pushes a
/// *second* frame back down to `stack.len() == 1`, so that frame is visited
/// exactly once per call regardless of how many of its own candidates get
/// tried — a `depth == 1` failure can provably never be looked up again, so
/// the immediate-mismatch recording below skips it rather than pay for an
/// `insert` (and, on a query that never backtracks at all, the memo's first
/// heap allocation) that nothing will ever read. Successes are never
/// memoized either: finding one short-circuits the whole search immediately
/// (`return true`), so there is never a later query for it to serve. Every
/// other `(candidate_id, depth)` pair is thus fully resolved (real work,
/// not a cache hit) at most once per [`match_combinator_chain`] call,
/// bounding the total number of [`match_from_element`] calls to a
/// low-degree polynomial in the ancestor/compound counts instead of
/// `2^depth`. The memo (`HashSet`) is created fresh per call and never
/// shared across calls — [`Combinator::Descendant`]'s search space for one
/// element/selector pair has no bearing on any other — and `HashSet::new()`
/// performs no heap allocation until the first `insert`, so the common
/// shape of a shallow selector (few combinators, hence few distinct
/// `depth` values) failing at its very first (`depth == 1`) combinator
/// check — e.g. `nav > a` where `nav` itself doesn't match anything —
/// touches the memo only via lookups against an empty set and pays nothing
/// beyond the empty struct. Selectors with more combinators that still
/// resolve without ever backtracking do perform a handful of `insert`s (at
/// most one per combinator actually walked, not `2^depth` of them) even
/// though nothing reads them back in that particular call.
///
/// This bound is not specific to [`Combinator::Descendant`]: the same memo
/// applies uniformly to every candidate examined in this function's loop,
/// regardless of which combinator produced it, so the identical
/// pathological shape on a [`Combinator::LaterSibling`] chain (`* ~ * ~ *
/// ~ ... ~ *` against a run of uniformly-matching siblings) is bounded the
/// same way, as a direct consequence rather than a separate fix.
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
        /// instead of cloning it. Negligible cost, no heap allocation; the
        /// broader allocation picture may be revisited in future perf
        /// work).
        iter: SelectorIter<'s, RaikiriSelectorImpl>,
        /// `(candidate_id, depth)` memo key this frame was pushed *for* —
        /// i.e. what to record as a proven dead end in `memo` once this
        /// frame's own candidates are exhausted without a match (see this
        /// function's "Memoization" doc). `None` only for the outermost
        /// frame (pushed directly from this function's own arguments, not
        /// from a candidate choice one level up — there is nothing to
        /// record a failure *against* for it, and its own exhaustion is
        /// this function's overall `false` return, not a sub-result any
        /// other choice point could ever re-query).
        origin: Option<(StyleNodeId, usize)>,
    }

    let mut memo: HashSet<(StyleNodeId, usize)> = HashSet::new();
    // The stack is seeded with one frame and gains one more per combinator
    // crossed. Building it as `vec![frame]` (the previous form) allocates
    // for exactly one element, so pushing a second frame — i.e. a selector
    // with 2+ combinators — already forces a reallocate-and-copy of every
    // frame so far. Reserving 4 slots up front covers up to 3 crossed
    // combinators (stack length 1 through 4) without regrowing; a selector
    // with 4 crossed combinators (5 frames) still regrows once, from
    // capacity 4 to 8.
    let mut stack = Vec::with_capacity(4);
    stack.push(Frame {
        candidates: pending_candidates_for(dom, combinator, current_id, ancestors),
        ancestors_unchanged: ancestors,
        iter,
        origin: None,
    });
    loop {
        // `stack.len()` at this point uniquely identifies "which compound
        // in the fixed original chain the frame about to be examined
        // introduced" — see this function's "Memoization" doc for why this
        // is a stable position marker independent of backtracking history.
        let depth = stack.len();
        let Some(frame) = stack.last_mut() else {
            // Outermost choice point exhausted with no full match found.
            return false;
        };
        let Some((candidate_id, candidate_ancestors)) =
            frame.candidates.next(dom, frame.ancestors_unchanged)
        else {
            // This level's candidates are exhausted — backtrack to the
            // parent choice point's next candidate. Everything this frame
            // could have tried has failed, so the candidate that led here
            // (if any — the outermost frame has none) is now a proven dead
            // end for any other backtracking path that reaches it too.
            let popped = stack.pop().expect("frame just borrowed via last_mut");
            if let Some(key) = popped.origin {
                memo.insert(key);
            }
            continue;
        };
        if memo.contains(&(candidate_id, depth)) {
            // A different backtracking path already proved this exact
            // candidate fails at this exact position in the chain — skip
            // straight to this frame's next candidate without redoing the
            // (possibly deep) exploration.
            continue;
        }
        let candidate_iter = frame.iter.clone();
        let Some(mut matched_iter) = match_from_element(
            dom,
            candidate_id,
            candidate_ancestors,
            candidate_iter,
            quirks_mode,
        ) else {
            // Candidate's compound didn't match — try this frame's next
            // candidate (loop back without push/pop). Immediate failure,
            // same as an exhausted pushed frame would record. Skipped at
            // `depth == 1`: that's the outermost frame, visited exactly
            // once per call (nothing ever pushes a second frame back down
            // to `stack.len() == 1`), so a depth-1 entry can provably never
            // be read back — recording it would just be a wasted `insert`
            // (and, on an otherwise retry-free failure, the first heap
            // allocation this memo would ever make).
            if depth != 1 {
                memo.insert((candidate_id, depth));
            }
            continue;
        };
        match matched_iter.next_sequence() {
            // No further combinator to the left: the whole complex
            // selector matched. Short-circuits immediately — nothing to
            // memoize, there is no later query this result could serve.
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
                    origin: Some((candidate_id, depth)),
                });
            }
        }
    }
}

/// Not-yet-tried candidates for one [`match_combinator_chain`] choice point
/// — the iterative counterpart of that function's four `match combinator`
/// arms' candidate-generation logic. Each variant
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
/// logic exactly — this function does no matching
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
        // cov:ignore: `Combinator::PseudoElement`/`SlotAssignment`/`Part`
        // are structurally unconstructible here.
        // The `selectors` crate only ever pushes each of these 3
        // combinators from behind its own `Parser` trait hook (parser.rs
        // `parse_one_simple_selector`), and `RaikiriSelectorParser`
        // overrides none of the three, so all fall to the trait's default:
        //   - `PseudoElement`: gated by `parse_pseudo_element`, default
        //     `Err`; also `RaikiriSelectorImpl::PseudoElement = PseudoElem`
        //     is an uninhabited enum, so no value could exist even if the
        //     hook were overridden to accept (verified via direct
        //     `a::before` parse).
        //   - `Part`: gated by `parse_part()`, default `false`.
        //   - `SlotAssignment`: gated by `parse_slotted()`, default
        //     `false`.
        // The `a::before` check only exercises the first hook — overriding
        // `parse_part`/`parse_slotted` on their own would make `Part`/
        // `SlotAssignment` reachable without `a::before` ever failing, so
        // overriding any of the three voids this exemption. See this
        // module's "他 combinator" doc note, above `match_combinator_chain`,
        // for the full argument. Yields an already-exhausted `Child(None)`
        // cursor — same "no candidate ever
        // succeeds" outcome the pre-fix `_ => false` arm produced.
        _ => PendingCandidates::Child(None),
    }
}

/// `parent_id`'s direct children (document order) が `Element` kind かつ
/// [`StyleNode::is_in_document`] であるかを判定する共有述語。
/// [`immediate_preceding_sibling`] と [`match_combinator_chain`] の
/// `LaterSibling` arm の両方から使う — [`collect_cascaded`] が
/// `ancestor_path` に積む前に行う `!node.is_in_document() => continue` gate
/// (同関数の doc 参照) と同じ基準を、sibling 側の候補選定でも揃えるための
/// 抽出 — 揃えないと `<template>` 子孫のような
/// inert element が sibling combinator の候補として拾われてしまう。
fn is_in_document_element<D: StyleDom>(dom: &D, id: StyleNodeId) -> bool {
    dom.node(id)
        .is_some_and(|node| node.is_in_document() && node.kind() == StyleNodeKind::Element)
}

/// `parent_id` の直接の子のうち、`current_id` の**直前**にいる element の id
/// ([`Combinator::NextSibling`] 用)。[`StyleDom::child_ids`] を先頭から 1
/// パス走査し、`current_id` に達した時点でそれまでに見た最後の element
/// candidate を返す — 割り当ては行わない (`Vec` 不使用、他 helper と
/// 同じ「使い捨て `Vec` を経由しない」方針を踏襲)。
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
/// combinator 由来かに依存しない (`ancestors` は
/// 兄弟ジャンプでは不変のまま引き継がれる — 兄弟は親を共有するため — ことが
/// この共有を成立させる。祖先ジャンプでは従来通り `split_last`/バックトラック
/// で truncate 済みの残り `ancestors` を渡す)。旧名 `match_from_ancestor`
/// — 兄弟候補にも使われるようになったため
/// `match_from_element` に rename。
///
/// 以前は、compound が一致した後さらに左の
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
    // `ancestors` for the rightmost compound —
    // needed so a structural pseudo-class in a non-rightmost compound
    // (e.g. `body > div:only-child p`) resolves against the right parent,
    // not `elem`'s (the search's original caller's) parent. Sibling jumps
    // pass `ancestors` through unchanged (siblings
    // share a parent), so this holds for those candidates too.
    if !compound_matches(dom, &mut iter, &elem, elem_id, ancestors, quirks_mode) {
        return None;
    }
    Some(iter)
}

// ---------------------------------------------------------------------------
// `:lang()` / `:dir()`.
//
// Both pseudo-classes resolve a property of the element that is NOT a plain
// own-attribute lookup — CSS Selectors L4 explicitly distinguishes them from
// the attribute-selector equivalent (`[lang|=C]` / `[dir=C]`) precisely
// because they consult "the UA's knowledge of the document's semantics"
// (`:dir()`'s own wording, quoted on `resolve_directionality`'s doc) —
// concretely, ancestor inheritance. Both therefore reuse the same
// `ancestors: &[StyleNodeId]` (root-first, immediate-parent-last) that
// `compound_matches` already threads through for descendant/child combinator
// matching — self is checked first, then
// `ancestors` is walked from `.last()` (immediate parent) toward `.first()`
// (document root).
// ---------------------------------------------------------------------------

/// `PseudoClass::Lang` arm of [`compound_matches`] — CSS Selectors L4 §7.2
/// <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>: "represents an
/// element whose content language is one of the languages listed in its
/// argument" (bikeshed source verbatim, see [`language_range_matches`] doc
/// for the fetch note). `ranges` is empty-or-more per [`crate::PseudoClass::Lang`]
/// grammar (`parse_comma_separated` never actually returns an empty `Vec`
/// for a non-empty `:lang(...)` argument list, but this function does not
/// special-case emptiness — `ranges.iter().any(..)` is vacuously `false` on
/// an empty slice, the same "never matches" outcome an empty argument list
/// should have, so no explicit guard is needed even if that upstream
/// guarantee ever changes).
///
/// [`effective_language`] always resolves to a concrete (possibly empty)
/// content language string — HTML LS §3.2.6.2's "determine the language of
/// a node" algorithm is a total function (its own final "Otherwise" step is
/// exactly [`effective_language`]'s fallback; see that function's doc), so
/// there is no "unresolvable language" case to handle here.
///
/// The empty string is a narrower case than a non-empty content language:
/// it does not match a bare wildcard range, but it does match other ranges, notably the
/// literal empty-string range `:lang("")`. CSS Selectors L4 §7.2, bikeshed
/// source `selectors-4/Overview.bs` `#the-lang-pseudo` (direct raw fetch of
/// `raw.githubusercontent.com/w3c/csswg-drafts/main/selectors-4/Overview.bs`,
/// bypassing WebFetch's truncation on this TR page the same way
/// [`matches_empty`]'s `:empty` doc note does), verbatim: "For this
/// purpose, a wildcard language range (\"*\") does not match elements
/// whose language is not tagged (e.g. `lang=\"\"`), but does match elements
/// whose language is tagged as undetermined (`lang=und`). A language range
/// consisting of an empty string (`:lang(\"\")`) matches (only) elements
/// whose language is not tagged." [`language_range_matches`] itself already
/// enforces both halves of this quote directly: it special-cases an empty
/// `content_language` (needed for `:lang("")`, which is a CSS-level
/// construct rather than a well-formed BCP47 range) by requiring exact
/// string equality against `range`, which yields `false` for a bare `*`
/// range against an empty `content_language` and `true` for `:lang("")`
/// against one — so this function needs no special case of its own and
/// simply delegates every range to it.
fn lang_pseudo_matches<D: StyleDom, E: StyleElement>(
    ranges: &[String],
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> bool {
    let lang = effective_language(dom, elem, ancestors);
    ranges
        .iter()
        .any(|range| language_range_matches(range, &lang))
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
/// unknown — represented, per the quoted "the corresponding language tag is
/// the empty string" fallback, as `String::new()` (see the "`lang=\"\"`
/// stopping inheritance" section below for how this converges with the
/// explicit-`lang=\"\"` case).
///
/// Like [`own_explicit_direction`]'s `dir` reads, the **own**-attribute
/// step gates on `elem.namespace_uri()` — but a 2-element allowlist (HTML
/// *or* SVG) rather than `dir`'s HTML-only 1-element one, per the quoted
/// step's explicit "an HTML element or an element in the SVG namespace"
/// wording (an earlier version of this
/// function read `lang` unconditionally, which is wrong for any other
/// foreign-namespace element — MathML concretely: `<math lang="ja">` nested
/// under `<html lang="en">` must resolve to `"en"`, not `"ja"`, since MathML
/// is neither HTML nor SVG. [`own_html_or_svg_lang_attribute`] is the gate;
/// see its doc for the allowlist). This only restricts *whose own*
/// attribute counts — the ancestor walk below still applies the same gate
/// per ancestor (a MathML ancestor's `lang` is skipped too, same as its own
/// element case), and a chain that bottoms out with no HTML/SVG element
/// carrying `lang` still resolves to `String::new()`, same as "absent
/// everywhere" below.
///
/// # Deliberately out of scope
///
/// - **`xml:lang` (XML-namespace `lang`)** — the first step in the quoted
///   list, and it *would* take priority over the plain `lang` attribute.
///   Skipped because raikiri does not parse XML/XHTML documents at all yet
///   (`StyleDom::quirks_mode` doc / `resolve_case_sensitivity` doc: "raikiri
///   は現時点で HTML document のみ対象") — there is no XML-namespace
///   attribute surface to read.
///
/// # `lang=""` stopping inheritance
///
/// Per the quoted algorithm, an empty-string `lang` attribute is itself a
/// *found* value ("the primary language is unknown", a distinct terminal
/// state from "no `lang` attribute at all", which keeps walking to the
/// parent). [`StyleElement::attr`] tracks attribute presence independent of
/// value, so [`own_html_or_svg_lang_attribute`] observes an explicit
/// `lang=""` as `Some("")`, not `None` — the `if let Some(lang) = ...`
/// branch below returns immediately for that case (yielding the empty
/// string) rather than falling through to the ancestor walk, matching the
/// quoted algorithm's step order. Callers must still treat this returned
/// empty string as "no content language" for their own purposes if that is
/// what they need (CSS Selectors L4's `:lang()` does — see
/// [`lang_pseudo_matches`]'s doc); [`effective_language`] itself only
/// resolves the language per HTML LS's algorithm, it does not decide what
/// an empty result means to a particular consumer.
///
/// The ancestor-chain-exhausted terminal case below (no `lang` found
/// anywhere) converges on this same empty-string representation, per the
/// quoted algorithm's own final "the corresponding language tag is the
/// empty string" fallback — even though it is reached via a different step
/// (running out of ancestors, not an explicit `lang=""` short-circuit).
fn effective_language<D: StyleDom, E: StyleElement>(
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> String {
    if let Some(lang) = own_html_or_svg_lang_attribute(elem) {
        return lang.to_owned();
    }
    for &ancestor_id in ancestors.iter().rev() {
        // `node`'s borrow must outlive `ancestor_elem`'s — a `.and_then`
        // chain would try to return a `&str` borrowed from a `node` that
        // drops at the end of the closure, hence the explicit `if let`
        // nesting instead of the more compact combinator chain
        // `effective_language`'s own doc-adjacent sibling functions use
        // where the borrow doesn't need to cross a temporary like this.
        if let Some(node) = dom.node(ancestor_id)
            && let Some(ancestor_elem) = node.as_element()
            && let Some(lang) = own_html_or_svg_lang_attribute(&ancestor_elem)
        {
            return lang.to_owned();
        }
    }
    // Ancestor chain exhausted with no pragma-set default / protocol-level
    // language available (both out of scope, see this function's doc).
    // HTML LS §3.2.6.2's final fallback: "the language of the node is
    // unknown, and the corresponding language tag is the empty string."
    String::new()
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
/// # BCP47 well-formedness and canonicalization
///
/// CSS Selectors L4 §7.2 additionally requires (bikeshed source, same fetch
/// as above):
///
/// > The \[content language\] and the \[language range\] must be
/// > canonicalized and converted to extlang form as per section 4.5 of
/// > \[RFC5646\] prior to the extended filtering operation; language tags or
/// > ranges that are not valid do not match anything. \[...\] The language
/// > range must be an extended language range according to BCP47. Language
/// > ranges that are not well-formed language tags or which would not be a
/// > well-formed language tag if an initial wildcard character "\*" were
/// > replaced with a valid subtag, do not match anything.
///
/// with an example spelling out that `:lang(åå)` "would not match, because
/// it contain\[s\] non-ASCII characters so is ill-formed", while `:lang(qq)`
/// "could match, even though qq is not a registered language code" — i.e.
/// the bar is grammatical **well-formedness** (RFC 5646 §2.1's ABNF), not
/// full **validity** (well-formed *and* every subtag registered in the IANA
/// Language Subtag Registry, RFC5646's own stricter term — `qq` is
/// well-formed but not valid, and the spec's own example says it can still
/// match).
///
/// [`is_well_formed_language_tag`] and [`is_well_formed_extended_language_range`]
/// implement well-formedness; [`canonicalize_primary_language_subtag`]
/// implements the canonicalization step that matters for matching
/// correctness (deprecated-subtag replacement). Both are applied to `range`
/// and `content_language` before the extended-filtering algorithm below
/// runs. RFC 5646 §4.5 canonicalization (deprecated subtag -> registry
/// `Preferred-Value`) is a genuine *false-negative* source: without it,
/// `language_range_matches("he", "iw")` returns `false` even though `iw` is
/// the deprecated form of `he` and a conformant UA must match `:lang(he)`
/// against `lang="iw"`.
///
/// ## Implemented subset
///
/// - **Well-formedness** is checked per-subtag against the shared
///   length/charset envelope every RFC 5646 §2.1 subtag production other
///   than the primary language subtag reduces to (1 to 8 ASCII letters or
///   digits — see [`is_well_formed_alphanum_subtag`]'s doc for the
///   derivation across `extlang`/`script`/`region`/`variant`/`extension`/
///   `privateuse`), plus a stricter first-subtag check for the `langtag`
///   alternative (ASCII alpha only, length bounds per position) and a
///   separate arm for the top-level `privateuse` alternative used alone
///   (`x-foo`) — see [`is_well_formed_language_tag`]'s doc for both. This is
///   **not** a full position-tracking walk of the `langtag` production:
///   subtag *sequencing* is not validated. Concretely, this implementation
///   does not detect `en-DE-Latn` (region before script), `en-Latn-Cyrl`
///   (two script-shaped subtags), `en-ab1` (a digit where only an
///   extlang/alpha subtag could legally appear), or a bare extension
///   singleton with no following value subtag (`en-a`) as ill-formed — each
///   is accepted because every individual subtag has *some* legal shape,
///   even though the sequence as a whole does not parse under the `langtag`
///   production. What this check *does* still reject beyond the shape
///   envelope: any `langtag`-shaped tag whose first subtag is alpha but
///   shorter than 2 characters — in practice this means 13 of RFC 5646's 26
///   fixed `irregular`/`regular` grandfathered tags (17 `irregular` + 9
///   `regular`) — the `i-*` ones (`i-klingon`, `i-navajo`, ...), whose
///   leading `i` subtag is 1 character. The remaining grandfathered tags
///   (`art-lojban`, `cel-gaulish`, `no-bok`, `no-nyn`, `zh-guoyu`,
///   `zh-hakka`, `zh-min`, `zh-min-nan`, `zh-xiang`, `en-GB-oed`,
///   `sgn-BE-FR`, `sgn-BE-NL`, `sgn-CH-DE`) all have a 2-or-more-character
///   alpha first subtag and so pass this check as ordinary well-formed
///   tags — their special, registration-defined meaning is not recognized
///   (see the canonicalization bullet below for the consequence of that).
/// - **Canonicalization** implements RFC 5646 §4.5 step 3 ("Subtags are
///   replaced by their 'Preferred-Value'"), restricted to the primary
///   language subtag and to a documented table
///   ([`DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS`]) rather than the full IANA
///   registry. Not implemented: extension-subtag reordering (§4.5 step 1),
///   grandfathered/redundant-tag replacement (§4.5 step 2 — the observable
///   consequence, given the well-formedness bullet above: `:lang(hak)`
///   does not match `lang="zh-hakka"`, and `:lang(nb)` does not match
///   `lang="no-bok"`, even though both grandfathered tags pass
///   well-formedness), and the extlang-form `Prefix`-restoration step
///   (needed only when a *primary* language subtag is itself a deprecated
///   extlang subtag — none of this table's entries are).
///
/// A full subtag registry / grammar checker (matching the scope the
/// Unicode Bidi Algorithm gets for `:dir()`'s `auto` value) remains a
/// substantial undertaking beyond this documented subset.
pub(crate) fn language_range_matches(range: &str, content_language: &str) -> bool {
    fn subtags_match(range_subtag: &str, tag_subtag: &str) -> bool {
        range_subtag == "*" || range_subtag.eq_ignore_ascii_case(tag_subtag)
    }
    fn is_singleton(subtag: &str) -> bool {
        subtag.chars().count() == 1
    }

    // `:lang("")` and an untagged content language (HTML LS §3.2.6.2's own
    // "the corresponding language tag is the empty string" fallback,
    // quoted on `effective_language`'s doc) are CSS-level constructs, not
    // BCP47 language tags/ranges — RFC 5646/4647 well-formedness and
    // canonicalization do not apply to either side here. Selectors L4 §7.2
    // (quoted in full on `lang_pseudo_matches`'s doc) instead gives them
    // its own equality rule directly: "A language range consisting of an
    // empty string matches (only) elements whose language is not tagged."
    // This equality check also subsumes "a wildcard language range does
    // not match elements whose language is not tagged" for every range
    // (not just a literal `*`) once `content_language` is empty, since no
    // non-empty range string can equal the empty string.
    if range.is_empty() || content_language.is_empty() {
        return range == content_language;
    }

    let range_subtags: Vec<&str> = range.split('-').collect();
    let tag_subtags: Vec<&str> = content_language.split('-').collect();

    // Selectors L4 §7.2 (quoted above): ill-formed tags/ranges never match.
    if !is_well_formed_extended_language_range(&range_subtags)
        || !is_well_formed_language_tag(&tag_subtags)
    {
        return false;
    }

    // Selectors L4 §7.2 (quoted above): canonicalize before extended
    // filtering. `subtags_match`/the loop below only ever read these
    // through `.eq_ignore_ascii_case`/`== "*"`, so owning `String`s here
    // (needed to overwrite the primary language subtag in place) costs
    // nothing but an allocation per subtag, on a selector-matching path
    // this crate does not treat as hot.
    let mut range_subtags: Vec<String> = range_subtags.into_iter().map(String::from).collect();
    let mut tag_subtags: Vec<String> = tag_subtags.into_iter().map(String::from).collect();
    canonicalize_primary_language_subtag(&mut range_subtags);
    canonicalize_primary_language_subtag(&mut tag_subtags);

    // Step 2: first subtag must match (range's first subtag may itself be
    // `*`, e.g. the bare wildcard range `:lang(*)` — `subtags_match` already
    // handles that).
    if !subtags_match(&range_subtags[0], &tag_subtags[0]) {
        return false;
    }
    let mut ri = 1;
    let mut ti = 1;

    // Step 3.
    while ri < range_subtags.len() {
        let r = &range_subtags[ri];
        if r == "*" {
            ri += 1; // 3.A
            continue;
        }
        let Some(t) = tag_subtags.get(ti) else {
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

/// Is `subtag` well-formed as any RFC 5646 §2.1 subtag production **other
/// than** the primary language subtag? `extlang` = `3ALPHA`, `script` =
/// `4ALPHA`, `region` = `2ALPHA / 3DIGIT`, `variant` = `5*8alphanum /
/// (DIGIT 3alphanum)`, an extension singleton = 1 alphanumeric character,
/// an extension value subtag = `2*8alphanum`, and a `privateuse`
/// introducer (`"x"`) or value subtag = `1*8alphanum`. Every one of these
/// productions falls inside the same envelope — 1 to 8 ASCII letters or
/// digits — so rather than tracking which specific production a subtag
/// belongs to (a full position-tracking grammar walk, out of scope per
/// [`language_range_matches`]'s doc), this function checks that shared
/// envelope directly.
fn is_well_formed_alphanum_subtag(subtag: &str) -> bool {
    (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Is `subtags` (already split on `-`, guaranteed non-empty by
/// [`language_range_matches`]'s empty-string early return) well-formed as a
/// BCP47 **`Language-Tag`**? RFC 5646 §2.1's top-level production is
/// `Language-Tag = langtag / privateuse / grandfathered`; this function
/// implements the first two alternatives (`grandfathered` is not
/// recognized as its own alternative — see [`language_range_matches`]'s
/// doc for which grandfathered tags this still accepts as ordinary
/// `langtag`s and which it rejects).
///
/// For `langtag`, the primary language subtag (`subtags[0]`) must be pure
/// ASCII alpha, 2 to 8 characters — the union of `language`'s three ABNF
/// alternatives (`2*3ALPHA`, the reserved `4ALPHA`, and `5*8ALPHA`). For the
/// top-level `privateuse` alternative (`"x" 1*("-" (1*8alphanum))`),
/// `subtags[0]` must case-insensitively equal `"x"` and at least one
/// further subtag must be present (the ABNF's `1*`); this is the only
/// signal this function uses to pick between the two alternatives, so it
/// cannot separately model a `langtag`'s own optional trailing
/// `privateuse` extension — that extension's subtags simply pass through
/// [`is_well_formed_alphanum_subtag`] like any other trailing subtag.
/// Every subtag after the first, in either alternative, only needs
/// [`is_well_formed_alphanum_subtag`]'s shared envelope; see that
/// function's doc, and [`language_range_matches`]'s doc for the list of
/// `langtag` subtag *sequences* this does not validate.
fn is_well_formed_language_tag(subtags: &[&str]) -> bool {
    let first = subtags[0];
    let first_ok = if first.eq_ignore_ascii_case("x") {
        subtags.len() >= 2
    } else {
        (2..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic())
    };
    first_ok
        && subtags[1..]
            .iter()
            .all(|s| is_well_formed_alphanum_subtag(s))
}

/// Is `subtags` (already split on `-`, guaranteed non-empty by
/// [`language_range_matches`]'s empty-string early return) well-formed as
/// an RFC 4647 §2.2 "extended language range"?
///
/// > extended-language-range = (1\*8ALPHA / "\*")
/// >                           \*("-" (1\*8alphanum / "\*"))
///
/// The first subtag must be ASCII alpha (1 to 8 characters) or the
/// wildcard `*`; every later subtag must be `*` or satisfy
/// [`is_well_formed_alphanum_subtag`]. This is RFC4647's own range grammar
/// (looser than [`is_well_formed_language_tag`]'s `langtag` production,
/// e.g. it has no per-position script/region/variant distinctions) rather
/// than an implementation of Selectors L4 §7.2's "would not be a
/// well-formed language tag if an initial wildcard \[...\] were replaced
/// with a valid subtag" clause, which would require backtracking over
/// every possible wildcard-to-subtag substitution; see
/// [`language_range_matches`]'s doc for the scope this leaves out.
fn is_well_formed_extended_language_range(subtags: &[&str]) -> bool {
    let first = subtags[0];
    let first_ok = first == "*"
        || ((1..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic()));
    first_ok
        && subtags[1..]
            .iter()
            .all(|&s| s == "*" || is_well_formed_alphanum_subtag(s))
}

/// The deprecated **2-letter** (`Type: language`) primary language subtags
/// from the IANA Language Subtag Registry
/// (<https://www.iana.org/assignments/language-subtag-registry/language-subtag-registry>)
/// — every registry record with `Type: language`, a subtag exactly 2
/// letters long, and both a `Deprecated` and a `Preferred-Value` field.
/// This is the complete set of *2-letter* deprecated primary language
/// subtags; the registry additionally lists over 100 deprecated
/// **3-letter** (ISO 639-3) primary language subtags — macrolanguage or
/// orthography mergers such as `ncp` -> `kdz` — which this table
/// deliberately excludes.
const DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS: &[(&str, &str)] = &[
    ("bh", "bih"),
    ("in", "id"),
    ("iw", "he"),
    ("ji", "yi"),
    ("jw", "jv"),
    ("mo", "ro"),
];

/// Canonicalizes `subtags`' primary language subtag (index 0; `subtags` is
/// guaranteed non-empty by [`language_range_matches`]'s empty-string early
/// return) to its registry `Preferred-Value` when it matches (ASCII
/// case-insensitively — subtags are case-insensitive per RFC 5646 §2.1.1)
/// one of [`DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS`], per RFC 5646 §4.5 step 3
/// ("Subtags are replaced by their 'Preferred-Value', if there is one").
/// The replacement is written out in the registry's own lowercase form,
/// which is harmless here because [`language_range_matches`]'s extended
/// filtering comparison is itself ASCII-case-insensitive.
///
/// Only the primary language subtag is ever replaced — this crate's
/// documented subset has no extlang, script, region, variant, extension,
/// or grandfathered/redundant-tag `Preferred-Value` entries, so RFC 5646
/// §4.5's other canonicalization steps are not implemented; see
/// [`language_range_matches`]'s doc for the list.
fn canonicalize_primary_language_subtag(subtags: &mut [String]) {
    for &(deprecated, preferred) in DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS {
        if subtags[0].eq_ignore_ascii_case(deprecated) {
            subtags[0] = preferred.to_string();
            break;
        }
    }
}

/// `PseudoClass::Dir` arm of [`compound_matches`] — resolves the element's
/// **directionality** per HTML Living Standard §3.2.6.4 "The `dir`
/// attribute" (<https://html.spec.whatwg.org/multipage/dom.html#the-directionality>)
/// simplified as documented in the sections below:
///
/// > The directionality of an element \[...\] is either 'ltr' or 'rtl'. To
/// > compute the directionality given an element element, switch on
/// > element's dir attribute state: LTR — Return 'ltr'. RTL — Return 'rtl'.
/// > \[...\] Auto \[...\] Let result be the auto directionality of element.
/// > If result is null, then return 'ltr'. Return result. \[...\] Undefined
/// > \[...\] Otherwise — Return the parent directionality of element.
/// >
/// > To compute the parent directionality given an element element: Let
/// > parentNode be element's parent node. \[...\] If parentNode is an
/// > element, then return the directionality of parentNode. Return 'ltr'.
///
/// i.e. own `dir="ltr"`/`dir="rtl"` wins; own `dir="auto"` uses
/// [`auto_directionality`]'s text scan (falling back to `'ltr'`, *not* the
/// parent's directionality, when the scan finds nothing); otherwise
/// (missing/invalid `dir`) walk up to the nearest ancestor with an explicit
/// `ltr`/`rtl` `dir`; if none exists anywhere (including at the document
/// root, which has no parent element), the default is `'ltr'` — this
/// function is total (`Direction`, not `Option<Direction>`), matching the
/// spec's own "always ltr or rtl, never undetermined" shape.
///
/// CSS Selectors L4 §7.1 <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>
/// (bikeshed source, same fetch as [`Direction`]'s doc) is what motivates
/// consulting ancestors at all rather than just the own attribute (`[dir=C]`
/// would suffice for that): "the directionality of an element inherits so
/// that a child without a dir attribute will have the same directionality
/// as its closest ancestor with a valid dir attribute."
///
/// # Deliberately out of scope
///
/// The quoted `Auto` arm above elides two branches of the referenced "auto
/// directionality" algorithm that this crate does not implement — see
/// [`auto_directionality`]'s own doc for the precise cut and why each is out
/// of scope. Also elided from the `Undefined` arm: the `bdi` element and
/// `input[type=tel]` special cases (raikiri has no notion of
/// element-specific behavior at this layer — `bdi` without an explicit
/// `dir` attribute still falls through to the ancestor walk below, same as
/// any other element with no `dir`) and the shadow-tree host step of
/// "parent directionality" (raikiri has no shadow DOM).
pub(crate) fn resolve_directionality<D: StyleDom, E: StyleElement>(
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
) -> Direction {
    match own_dir_attribute_state(elem) {
        DirAttributeState::Ltr => return Direction::Ltr,
        DirAttributeState::Rtl => return Direction::Rtl,
        DirAttributeState::Auto => {
            return auto_directionality(dom, elem_id).unwrap_or(Direction::Ltr);
        }
        DirAttributeState::Undefined => {}
    }
    for &ancestor_id in ancestors.iter().rev() {
        // Same "explicit `if let` nesting instead of `.and_then` chain"
        // reason as `effective_language`'s sibling loop — `own_explicit_direction`
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

/// The `dir` attribute's full enumerated state — HTML LS §3.2.6.4's LTR /
/// RTL / Auto / Undefined states (same fetch as [`resolve_directionality`]'s
/// doc) — including `Auto`, which [`own_explicit_direction`] (the
/// ancestor-walk helper, which only ever needs an explicit `ltr`/`rtl`
/// winner) folds into `None` alongside `Undefined`.
///
/// Attribute keyword matching is ASCII case-insensitive, per HTML's general
/// treatment of enumerated attribute keywords (`the dir attribute is an
/// enumerated attribute with the following keywords and states`, same
/// fetch) — same posture as this crate's other HTML-enumerated-value
/// comparisons (e.g. `elem.tag_name()`'s `eq_ignore_ascii_case` in
/// [`compound_matches`]). Missing or invalid values (anything other than
/// `ltr`/`rtl`/`auto`) both resolve to `Undefined` per HTML LS's own
/// "missing value default and invalid value default are both the Undefined
/// state".
///
/// # HTML-namespace-only, unlike [`effective_language`]'s `lang` reads
///
/// The same fetch continues, immediately after the quoted algorithm:
///
/// > Since the `dir` attribute is only defined for HTML elements, it cannot
/// > be present on elements from other namespaces. Thus, elements from
/// > other namespaces always end up using the parent directionality.
///
/// so a `dir` attribute on a foreign-namespace element (e.g. inline
/// `<svg dir="rtl">`) must resolve to `Undefined` here regardless of its
/// literal value — `elem.namespace_uri().is_none()` is this crate's
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirAttributeState {
    /// `dir="ltr"` — HTML LS "LTR" state.
    Ltr,
    /// `dir="rtl"` — HTML LS "RTL" state.
    Rtl,
    /// `dir="auto"` — HTML LS "Auto" state.
    Auto,
    /// Missing, invalid, or on a non-HTML-namespace element — HTML LS
    /// "Undefined" state.
    Undefined,
}

fn own_dir_attribute_state<E: StyleElement>(elem: &E) -> DirAttributeState {
    if elem.namespace_uri().is_some() {
        return DirAttributeState::Undefined;
    }
    match elem.attr("dir") {
        Some(v) if v.eq_ignore_ascii_case("ltr") => DirAttributeState::Ltr,
        Some(v) if v.eq_ignore_ascii_case("rtl") => DirAttributeState::Rtl,
        Some(v) if v.eq_ignore_ascii_case("auto") => DirAttributeState::Auto,
        _ => DirAttributeState::Undefined,
    }
}

/// HTML's `dir` attribute LTR/RTL states only — `Undefined`/`Auto` (missing,
/// invalid, or `auto`) both collapse to `None` here, since the only caller
/// ([`resolve_directionality`]'s ancestor walk) needs exactly "does this
/// ancestor have an explicit winner to inherit", and per HTML LS's "parent
/// directionality" an ancestor's own `Auto`/`Undefined` state is not such a
/// winner (the walk keeps going past it toward that ancestor's own parent).
/// See [`own_dir_attribute_state`]'s doc for the full 4-state read this
/// delegates to, including the namespace gate and case-insensitivity.
fn own_explicit_direction<E: StyleElement>(elem: &E) -> Option<Direction> {
    match own_dir_attribute_state(elem) {
        DirAttributeState::Ltr => Some(Direction::Ltr),
        DirAttributeState::Rtl => Some(Direction::Rtl),
        DirAttributeState::Auto | DirAttributeState::Undefined => None,
    }
}

/// HTML LS §3.2.6.4's "auto directionality" given an element (same fetch as
/// [`resolve_directionality`]'s doc), simplified to the text-content-scan
/// branch only:
///
/// > To compute the auto directionality given an element element: \[...\]
/// > Return the contained text auto directionality of element with
/// > canExcludeRoot set to false.
///
/// i.e. [`contained_text_auto_directionality`] over `element`'s own
/// descendants, `None` iff no descendant text node contains a strong L/AL/R
/// character anywhere ([`resolve_directionality`]'s caller then applies the
/// spec's own `'ltr'` fallback for that case).
///
/// # Deliberately out of scope
///
/// The full algorithm has two branches elided here, both unreachable from
/// this crate's current DOM model:
///
/// - **Form-associated elements' current value** — the spec's first step
///   scans an `input`/`textarea` element's live *value* (not its descendant
///   text) for the same L/AL/R rule. [`StyleElement`] has no notion of a
///   form control's current value (a run-time, potentially
///   script-or-user-mutated state distinct from the parsed DOM tree this
///   crate's style layer operates over) — out of scope for the same reason
///   [`crate::style_dom`] carries no form-control surface at all. Falling
///   through to the descendant-text scan instead happens to coincide with a
///   `textarea`'s *initial* value (its child text node) but not any
///   subsequently-changed value, and always resolves `input` (a void
///   element with no descendant text) to `None` regardless of what the
///   user has typed into it.
/// - **`slot` elements with shadow-tree assigned nodes** — raikiri has no
///   shadow DOM (established elsewhere, e.g. [`resolve_directionality`]'s
///   own "Deliberately out of scope" note on the shadow-tree host step of
///   "parent directionality"), so this branch's precondition never holds.
fn auto_directionality<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> Option<Direction> {
    contained_text_auto_directionality(dom, elem_id)
}

/// HTML LS §3.2.6.4's "contained text auto directionality" of an element,
/// with `canExcludeRoot` fixed to `false` (the only value
/// [`auto_directionality`] ever needs — `true` is only used by the
/// shadow-tree `slot` branch this crate does not implement, see that
/// function's doc). Same fetch as [`resolve_directionality`]'s doc:
///
/// > For each node descendant of element's descendants, in tree order: If
/// > any of \[descendant, any ancestor element of descendant that is a
/// > descendant of element\] is one of \[a bdi element, a script element, a
/// > style element, a textarea element, an element whose dir attribute is
/// > not in the Undefined state\], then continue. \[...\] If descendant is
/// > not a Text node, then continue. Let result be the text node
/// > directionality of descendant. If result is not null, then return
/// > result. \[...\] Return null.
///
/// i.e. a depth-first, tree-order walk of `elem_id`'s descendants that skips
/// entire subtrees rooted at a `bdi`/`script`/`style`/`textarea` element or
/// any element with its own non-`Undefined` `dir` (those resolve their own
/// directionality independently and must not contribute text to this
/// element's scan — see [`excluded_from_auto_text_scan`]), scanning every
/// remaining text node in order via [`text_node_first_strong_direction`]
/// until one yields a non-`None` result. `elem_id` itself is never checked
/// against the exclusion set (that is exactly what `canExcludeRoot = false`
/// means) — only its descendants are.
///
/// This crate has no `slot` element / shadow-tree concept, so the spec's
/// "if descendant is a `slot` element whose root is a shadow root" branch
/// (which would otherwise interrupt this walk to consult a shadow host) is
/// omitted; every descendant that isn't excluded is either scanned (if a
/// `Text` node) or recursed into (if an `Element`).
fn contained_text_auto_directionality<D: StyleDom>(
    dom: &D,
    elem_id: StyleNodeId,
) -> Option<Direction> {
    dom.child_ids(elem_id)
        .find_map(|child_id| auto_text_scan_subtree(dom, child_id))
}

/// One node of [`contained_text_auto_directionality`]'s tree-order walk —
/// recurses into element subtrees (unless [`excluded_from_auto_text_scan`]),
/// and applies [`text_node_first_strong_direction`] to text nodes. Comment /
/// processing-instruction / document-fragment / document nodes are none of
/// "text node" or "element node" and so never contribute, matching how
/// [`matches_empty`] / [`is_substantial_node`] treat the same node kinds.
fn auto_text_scan_subtree<D: StyleDom>(dom: &D, node_id: StyleNodeId) -> Option<Direction> {
    let node = dom.node(node_id)?;
    match node.kind() {
        StyleNodeKind::Text => text_node_first_strong_direction(node.text_content().unwrap_or("")),
        StyleNodeKind::Element => {
            // cov:ignore: `kind() == Element` guarantees `as_element()` is
            // `Some` — `StyleNode::as_element`'s own trait doc contract
            // ("Some iff kind() == Element").
            let elem = node.as_element()?;
            if excluded_from_auto_text_scan(&elem) {
                return None;
            }
            dom.child_ids(node_id)
                .find_map(|child_id| auto_text_scan_subtree(dom, child_id))
        }
        StyleNodeKind::Comment
        | StyleNodeKind::ProcessingInstruction
        | StyleNodeKind::DocumentFragment
        | StyleNodeKind::Document => None,
    }
}

/// The exclusion set [`contained_text_auto_directionality`]'s spec quote
/// lists — an element that must not contribute (nor let its own
/// descendants contribute) text to an ancestor's auto-directionality scan,
/// because it resolves its own directionality independently: `bdi`,
/// `script`, `style`, `textarea`, or any element whose own `dir` attribute
/// is not `Undefined` (`ltr`/`rtl`/`auto` all count — an `auto` descendant
/// resolves its *own* text scan rather than leaking its text into the
/// ancestor's).
///
/// The 4 tag names are gated to the HTML namespace
/// (`elem.namespace_uri().is_none()`, this crate's established "is an HTML
/// element" proxy — see [`own_dir_attribute_state`]'s doc) since HTML LS's
/// `bdi`/`script`/`style`/`textarea` dfns denote specifically the HTML
/// elements of those names, not e.g. SVG's own distinct `script`/`style`
/// elements. The `dir`-attribute-state check needs no separate namespace
/// gate — [`own_dir_attribute_state`] already resolves any foreign-namespace
/// element to `Undefined`.
fn excluded_from_auto_text_scan<E: StyleElement>(elem: &E) -> bool {
    if !matches!(own_dir_attribute_state(elem), DirAttributeState::Undefined) {
        return true;
    }
    if elem.namespace_uri().is_some() {
        return false;
    }
    const EXCLUDED_TAGS: &[&str] = &["bdi", "script", "style", "textarea"];
    EXCLUDED_TAGS
        .iter()
        .any(|tag| elem.tag_name().eq_ignore_ascii_case(tag))
}

/// Bidirectional character type, restricted to the 3 *strong* types HTML
/// LS's "text node directionality" algorithm consults — see
/// [`text_node_first_strong_direction`]'s doc for the quoted algorithm and
/// [`strong_bidi_type`]'s doc for how a code point is classified into one
/// of these (or neither).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StrongBidiType {
    /// Left-to-Right.
    L,
    /// Arabic Letter.
    Al,
    /// Right-to-Left (non-Arabic).
    R,
}

/// HTML LS §3.2.6.4's "text node directionality" given a `Text` node's data
/// (same fetch as [`resolve_directionality`]'s doc):
///
/// > If text's data does not contain a code point whose bidirectional
/// > character type is L, AL, or R, then return null. Let codePoint be the
/// > first code point in text's data whose bidirectional character type is
/// > L, AL, or R. If codePoint is of bidirectional character type AL or R,
/// > then return 'rtl'. If codePoint is of bidirectional character type L,
/// > then return 'ltr'.
///
/// i.e. scan `data` in order for the first code point [`strong_bidi_type`]
/// classifies as L/AL/R (code points of any other — weak or neutral —
/// bidirectional type, which this crate does not otherwise classify at all,
/// are simply skipped over); `AL`/`R` resolve to `Direction::Rtl`, `L` to
/// `Direction::Ltr`; `None` iff no such code point exists anywhere in
/// `data`.
fn text_node_first_strong_direction(data: &str) -> Option<Direction> {
    data.chars().find_map(|c| match strong_bidi_type(c)? {
        StrongBidiType::L => Some(Direction::Ltr),
        StrongBidiType::Al | StrongBidiType::R => Some(Direction::Rtl),
    })
}

// Transcribed from `DerivedBidiClass.txt`'s "@missing" default-value
// comments for the code point ranges reserved for right-to-left
// scripts (see this function's doc) — each left edge/right edge here
// is one of those file's own `@missing: START..END; Bidi_Class` lines.
const AL_RANGES: &[(u32, u32)] = &[
    (0x0600, 0x07BF),   // Arabic, Syriac, Arabic Supplement, Thaana
    (0x0860, 0x08FF),   // Syriac Supplement, Arabic Extended-B/-A
    (0xFB50, 0xFDCF),   // Arabic Presentation Forms-A (partial)
    (0xFDF0, 0xFDFF),   // Arabic Presentation Forms-A (partial)
    (0xFE70, 0xFEFF),   // Arabic Presentation Forms-B
    (0x10D00, 0x10D3F), // Hanifi Rohingya
    (0x10EC0, 0x10EFF), // Arabic Extended-C
    (0x10F30, 0x10F6F), // Sogdian
    (0x1EC70, 0x1ECBF), // Indic Siyaq Numbers
    (0x1ED00, 0x1ED4F), // Ottoman Siyaq Numbers
    (0x1EE00, 0x1EEFF), // Arabic Mathematical Alphabetic Symbols
];
// FB1D..FB4F (Hebrew Presentation Forms) is a separate range from the
// main Hebrew block below because it is only *half* of the Alphabetic
// Presentation Forms block — the other half, FB00..FB1C (Latin
// ligatures), is Left_To_Right, not Right_To_Left.
const R_RANGES: &[(u32, u32)] = &[
    (0x0590, 0x05FF),   // Hebrew
    (0x07C0, 0x085F),   // NKo, Samaritan, Mandaic
    (0xFB1D, 0xFB4F),   // Hebrew Presentation Forms
    (0x10800, 0x10CFF), // Cypriot..Old Hungarian
    (0x10D40, 0x10EBF), // Garay, Rumi Numeral Symbols, Yezidi
    (0x10F00, 0x10F2F), // Old Sogdian
    (0x10F70, 0x10FFF), // Old Uyghur..Elymaic
    (0x1E800, 0x1EC6F), // Mende Kikakui..Adlam
    (0x1ECC0, 0x1ECFF), // reserved remainder after Indic Siyaq Numbers (R-default)
    (0x1ED50, 0x1EDFF), // reserved remainder after Ottoman Siyaq Numbers (R-default)
    (0x1EF00, 0x1EFFF), // reserved remainder after Arabic Mathematical Alphabetic Symbols (R-default)
];
// Code points *inside* an `AL_RANGES`/`R_RANGES` span above whose real
// `Bidi_Class` (per `DerivedBidiClass.txt`'s explicit, non-`@missing`
// per-code-point entries, Unicode 17.0.0) is not that span's own
// `AL`/`R` default — see this function's doc, the "known imprecision"
// paragraph, for why this table exists (over-classifying these as
// strong `AL`/`R` can resolve the wrong direction outright, not just
// stop the scan one code point early) and how it was derived. Checked
// before either range table below, so every code point here is
// classified `None` (non-strong) instead of falling into
// `AL_RANGES`/`R_RANGES`'s membership check.
const NON_STRONG_WITHIN_AL_R_RANGES: &[(u32, u32)] = &[
    // Hebrew
    (0x0591, 0x05BD), // NSM
    (0x05BF, 0x05BF), // NSM
    (0x05C1, 0x05C2), // NSM
    (0x05C4, 0x05C5), // NSM
    (0x05C7, 0x05C7), // NSM
    // Arabic, Syriac, Arabic Supplement, Thaana
    (0x0600, 0x0605), // AN
    (0x0606, 0x0607), // ON
    (0x0609, 0x060A), // ET
    (0x060C, 0x060C), // CS
    (0x060E, 0x060F), // ON
    (0x0610, 0x061A), // NSM
    (0x064B, 0x065F), // NSM
    (0x0660, 0x0669), // AN
    (0x066A, 0x066A), // ET
    (0x066B, 0x066C), // AN
    (0x0670, 0x0670), // NSM
    (0x06D6, 0x06DC), // NSM
    (0x06DD, 0x06DD), // AN
    (0x06DE, 0x06DE), // ON
    (0x06DF, 0x06E4), // NSM
    (0x06E7, 0x06E8), // NSM
    (0x06E9, 0x06E9), // ON
    (0x06EA, 0x06ED), // NSM
    (0x06F0, 0x06F9), // EN
    (0x0711, 0x0711), // NSM
    (0x0730, 0x074A), // NSM
    (0x07A6, 0x07B0), // NSM
    // NKo, Samaritan, Mandaic
    (0x07EB, 0x07F3), // NSM
    (0x07F6, 0x07F6), // ON
    (0x07F7, 0x07F9), // ON
    (0x07FD, 0x07FD), // NSM
    (0x0816, 0x0819), // NSM
    (0x081B, 0x0823), // NSM
    (0x0825, 0x0827), // NSM
    (0x0829, 0x082D), // NSM
    (0x0859, 0x085B), // NSM
    // Syriac Supplement, Arabic Extended-B/-A
    (0x0890, 0x0891), // AN
    (0x0897, 0x089F), // NSM
    (0x08CA, 0x08E1), // NSM
    (0x08E2, 0x08E2), // AN
    (0x08E3, 0x08FF), // NSM (this entry's real DerivedBidiClass.txt
    // range continues to U+0902, already covered separately by
    // `NON_STRONG_ALPHABETIC_RANGES`'s Devanagari `(0x0900, 0x0902)`
    // entry below — clipped here to stay inside this span)
    // Hebrew Presentation Forms
    (0xFB1E, 0xFB1E), // NSM
    (0xFB29, 0xFB29), // ES
    // Arabic Presentation Forms-A (partial)
    (0xFBC3, 0xFBD2), // ON
    (0xFD3E, 0xFD3E), // ON
    (0xFD3F, 0xFD3F), // ON
    (0xFD40, 0xFD4F), // ON
    (0xFD90, 0xFD91), // ON
    (0xFDC8, 0xFDCF), // ON
    (0xFDFD, 0xFDFF), // ON
    // Arabic Presentation Forms-B
    (0xFEFF, 0xFEFF), // BN
    // Cypriot..Old Hungarian
    (0x1091F, 0x1091F), // ON
    (0x10A01, 0x10A03), // NSM
    (0x10A05, 0x10A06), // NSM
    (0x10A0C, 0x10A0F), // NSM
    (0x10A38, 0x10A3A), // NSM
    (0x10A3F, 0x10A3F), // NSM
    (0x10AE5, 0x10AE6), // NSM
    (0x10B39, 0x10B3F), // ON
    // Hanifi Rohingya
    (0x10D24, 0x10D27), // NSM
    (0x10D30, 0x10D39), // AN
    // Garay, Rumi Numeral Symbols, Yezidi
    (0x10D40, 0x10D49), // AN
    (0x10D69, 0x10D6D), // NSM
    (0x10D6E, 0x10D6E), // ON
    (0x10E60, 0x10E7E), // AN
    (0x10EAB, 0x10EAC), // NSM
    // Arabic Extended-C
    (0x10ED0, 0x10ED0), // ON
    (0x10ED1, 0x10ED8), // ON
    (0x10EFA, 0x10EFF), // NSM
    // Sogdian
    (0x10F46, 0x10F50), // NSM
    // Old Uyghur..Elymaic
    (0x10F82, 0x10F85), // NSM
    // Mende Kikakui..Adlam
    (0x1E8D0, 0x1E8D6), // NSM
    (0x1E944, 0x1E94A), // NSM
    // Arabic Mathematical Alphabetic Symbols
    (0x1EEF0, 0x1EEF1), // ON
];
// Code points where `Alphabetic=Yes` (the property `char::is_alphabetic`
// implements) but the real `Bidi_Class` is something other than `L` —
// see this function's doc, "`is_alphabetic()` fallback" section, case 1,
// for why this table exists and how it was derived. None of these
// ranges overlaps `AL_RANGES`/`R_RANGES` above (by construction of the
// derivation), so every entry here really does fall through to the
// `is_alphabetic()` branch below absent this check.
const NON_STRONG_ALPHABETIC_RANGES: &[(u32, u32)] = &[
    // Spacing Modifier Letters
    (0x02B9, 0x02BA), // ON
    (0x02C6, 0x02CF), // ON
    (0x02EC, 0x02EC), // ON
    // Combining Diacritical Marks
    (0x0345, 0x0345), // NSM
    (0x0363, 0x036F), // NSM
    // Greek and Coptic
    (0x0374, 0x0374), // ON
    // Devanagari
    (0x0900, 0x0902), // NSM
    (0x093A, 0x093A), // NSM
    (0x0941, 0x0948), // NSM
    (0x0955, 0x0957), // NSM
    (0x0962, 0x0963), // NSM
    // Bengali
    (0x0981, 0x0981), // NSM
    (0x09C1, 0x09C4), // NSM
    (0x09E2, 0x09E3), // NSM
    // Gurmukhi
    (0x0A01, 0x0A02), // NSM
    (0x0A41, 0x0A42), // NSM
    (0x0A47, 0x0A48), // NSM
    (0x0A4B, 0x0A4C), // NSM
    (0x0A51, 0x0A51), // NSM
    (0x0A70, 0x0A71), // NSM
    (0x0A75, 0x0A75), // NSM
    // Gujarati
    (0x0A81, 0x0A82), // NSM
    (0x0AC1, 0x0AC5), // NSM
    (0x0AC7, 0x0AC8), // NSM
    (0x0AE2, 0x0AE3), // NSM
    (0x0AFA, 0x0AFC), // NSM
    // Oriya
    (0x0B01, 0x0B01), // NSM
    (0x0B3F, 0x0B3F), // NSM
    (0x0B41, 0x0B44), // NSM
    (0x0B56, 0x0B56), // NSM
    (0x0B62, 0x0B63), // NSM
    // Tamil
    (0x0B82, 0x0B82), // NSM
    (0x0BC0, 0x0BC0), // NSM
    // Telugu
    (0x0C00, 0x0C00), // NSM
    (0x0C04, 0x0C04), // NSM
    (0x0C3E, 0x0C40), // NSM
    (0x0C46, 0x0C48), // NSM
    (0x0C4A, 0x0C4C), // NSM
    (0x0C55, 0x0C56), // NSM
    (0x0C62, 0x0C63), // NSM
    // Kannada
    (0x0C81, 0x0C81), // NSM
    (0x0CCC, 0x0CCC), // NSM
    (0x0CE2, 0x0CE3), // NSM
    // Malayalam
    (0x0D00, 0x0D01), // NSM
    (0x0D41, 0x0D44), // NSM
    (0x0D62, 0x0D63), // NSM
    // Sinhala
    (0x0D81, 0x0D81), // NSM
    (0x0DD2, 0x0DD4), // NSM
    (0x0DD6, 0x0DD6), // NSM
    // Thai
    (0x0E31, 0x0E31), // NSM
    (0x0E34, 0x0E3A), // NSM
    (0x0E4D, 0x0E4D), // NSM
    // Lao
    (0x0EB1, 0x0EB1), // NSM
    (0x0EB4, 0x0EB9), // NSM
    (0x0EBB, 0x0EBC), // NSM
    (0x0ECD, 0x0ECD), // NSM
    // Tibetan
    (0x0F71, 0x0F7E), // NSM
    (0x0F80, 0x0F83), // NSM
    (0x0F8D, 0x0F97), // NSM
    (0x0F99, 0x0FBC), // NSM
    // Myanmar
    (0x102D, 0x1030), // NSM
    (0x1032, 0x1036), // NSM
    (0x103D, 0x103E), // NSM
    (0x1058, 0x1059), // NSM
    (0x105E, 0x1060), // NSM
    (0x1071, 0x1074), // NSM
    (0x1082, 0x1082), // NSM
    (0x1085, 0x1086), // NSM
    (0x108D, 0x108D), // NSM
    (0x109D, 0x109D), // NSM
    // Tagalog
    (0x1712, 0x1713), // NSM
    // Hanunoo
    (0x1732, 0x1733), // NSM
    // Buhid
    (0x1752, 0x1753), // NSM
    // Tagbanwa
    (0x1772, 0x1773), // NSM
    // Khmer
    (0x17B7, 0x17BD), // NSM
    (0x17C6, 0x17C6), // NSM
    // Mongolian
    (0x1885, 0x1886), // NSM
    (0x18A9, 0x18A9), // NSM
    // Limbu
    (0x1920, 0x1922), // NSM
    (0x1927, 0x1928), // NSM
    (0x1932, 0x1932), // NSM
    // Buginese
    (0x1A17, 0x1A18), // NSM
    (0x1A1B, 0x1A1B), // NSM
    // Tai Tham
    (0x1A56, 0x1A56), // NSM
    (0x1A58, 0x1A5E), // NSM
    (0x1A62, 0x1A62), // NSM
    (0x1A65, 0x1A6C), // NSM
    (0x1A73, 0x1A74), // NSM
    // Combining Diacritical Marks Extended
    (0x1ABF, 0x1AC0), // NSM
    (0x1ACC, 0x1ACE), // NSM
    // Balinese
    (0x1B00, 0x1B03), // NSM
    (0x1B36, 0x1B3A), // NSM
    (0x1B3C, 0x1B3C), // NSM
    (0x1B42, 0x1B42), // NSM
    // Sundanese
    (0x1B80, 0x1B81), // NSM
    (0x1BA2, 0x1BA5), // NSM
    (0x1BA8, 0x1BA9), // NSM
    (0x1BAC, 0x1BAD), // NSM
    // Batak
    (0x1BE8, 0x1BE9), // NSM
    (0x1BED, 0x1BED), // NSM
    (0x1BEF, 0x1BF1), // NSM
    // Lepcha
    (0x1C2C, 0x1C33), // NSM
    (0x1C36, 0x1C36), // NSM
    // Combining Diacritical Marks Supplement
    (0x1DD3, 0x1DF4), // NSM
    // Cyrillic Extended-A
    (0x2DE0, 0x2DFF), // NSM
    // Supplemental Punctuation
    (0x2E2F, 0x2E2F), // ON
    // Cyrillic Extended-B
    (0xA674, 0xA67B), // NSM
    (0xA67F, 0xA67F), // ON
    (0xA69E, 0xA69F), // NSM
    // Modifier Tone Letters
    (0xA717, 0xA71F), // ON
    // Latin Extended-D
    (0xA788, 0xA788), // ON
    // Syloti Nagri
    (0xA802, 0xA802), // NSM
    (0xA80B, 0xA80B), // NSM
    (0xA825, 0xA826), // NSM
    // Saurashtra
    (0xA8C5, 0xA8C5), // NSM
    // Devanagari Extended
    (0xA8FF, 0xA8FF), // NSM
    // Kayah Li
    (0xA926, 0xA92A), // NSM
    // Rejang
    (0xA947, 0xA951), // NSM
    // Javanese
    (0xA980, 0xA982), // NSM
    (0xA9B6, 0xA9B9), // NSM
    (0xA9BC, 0xA9BD), // NSM
    // Myanmar Extended-B
    (0xA9E5, 0xA9E5), // NSM
    // Cham
    (0xAA29, 0xAA2E), // NSM
    (0xAA31, 0xAA32), // NSM
    (0xAA35, 0xAA36), // NSM
    (0xAA43, 0xAA43), // NSM
    (0xAA4C, 0xAA4C), // NSM
    // Myanmar Extended-A
    (0xAA7C, 0xAA7C), // NSM
    // Tai Viet
    (0xAAB0, 0xAAB0), // NSM
    (0xAAB2, 0xAAB4), // NSM
    (0xAAB7, 0xAAB8), // NSM
    (0xAABE, 0xAABE), // NSM
    // Meetei Mayek Extensions
    (0xAAEC, 0xAAED), // NSM
    // Meetei Mayek
    (0xABE5, 0xABE5), // NSM
    (0xABE8, 0xABE8), // NSM
    // Ancient Greek Numbers
    (0x10140, 0x10174), // ON
    // Old Permic
    (0x10376, 0x1037A), // NSM
    // Brahmi
    (0x11001, 0x11001), // NSM
    (0x11038, 0x11045), // NSM
    (0x11073, 0x11074), // NSM
    // Kaithi
    (0x11080, 0x11081), // NSM
    (0x110B3, 0x110B6), // NSM
    (0x110C2, 0x110C2), // NSM
    // Chakma
    (0x11100, 0x11102), // NSM
    (0x11127, 0x1112B), // NSM
    (0x1112D, 0x11132), // NSM
    // Sharada
    (0x11180, 0x11181), // NSM
    (0x111B6, 0x111BE), // NSM
    (0x111CF, 0x111CF), // NSM
    // Khojki
    (0x1122F, 0x11231), // NSM
    (0x11234, 0x11234), // NSM
    (0x11237, 0x11237), // NSM
    (0x1123E, 0x1123E), // NSM
    (0x11241, 0x11241), // NSM
    // Khudawadi
    (0x112DF, 0x112DF), // NSM
    (0x112E3, 0x112E8), // NSM
    // Grantha
    (0x11300, 0x11301), // NSM
    (0x11340, 0x11340), // NSM
    // Tulu-Tigalari
    (0x113BB, 0x113C0), // NSM
    // Newa
    (0x11438, 0x1143F), // NSM
    (0x11443, 0x11444), // NSM
    // Tirhuta
    (0x114B3, 0x114B8), // NSM
    (0x114BA, 0x114BA), // NSM
    (0x114BF, 0x114C0), // NSM
    // Siddham
    (0x115B2, 0x115B5), // NSM
    (0x115BC, 0x115BD), // NSM
    (0x115DC, 0x115DD), // NSM
    // Modi
    (0x11633, 0x1163A), // NSM
    (0x1163D, 0x1163D), // NSM
    (0x11640, 0x11640), // NSM
    // Takri
    (0x116AB, 0x116AB), // NSM
    (0x116AD, 0x116AD), // NSM
    (0x116B0, 0x116B5), // NSM
    // Ahom
    (0x1171D, 0x1171D), // NSM
    (0x1171F, 0x1171F), // NSM
    (0x11722, 0x11725), // NSM
    (0x11727, 0x1172A), // NSM
    // Dogra
    (0x1182F, 0x11837), // NSM
    // Dives Akuru
    (0x1193B, 0x1193C), // NSM
    // Nandinagari
    (0x119D4, 0x119D7), // NSM
    (0x119DA, 0x119DB), // NSM
    // Zanabazar Square
    (0x11A01, 0x11A06), // NSM
    (0x11A09, 0x11A0A), // NSM
    (0x11A35, 0x11A38), // NSM
    (0x11A3B, 0x11A3E), // NSM
    // Soyombo
    (0x11A51, 0x11A56), // NSM
    (0x11A59, 0x11A5B), // NSM
    (0x11A8A, 0x11A96), // NSM
    // Sharada Supplement
    (0x11B60, 0x11B60), // NSM
    (0x11B62, 0x11B64), // NSM
    (0x11B66, 0x11B66), // NSM
    // Bhaiksuki
    (0x11C30, 0x11C36), // NSM
    (0x11C38, 0x11C3D), // NSM
    // Marchen
    (0x11C92, 0x11CA7), // NSM
    (0x11CAA, 0x11CB0), // NSM
    (0x11CB2, 0x11CB3), // NSM
    (0x11CB5, 0x11CB6), // NSM
    // Masaram Gondi
    (0x11D31, 0x11D36), // NSM
    (0x11D3A, 0x11D3A), // NSM
    (0x11D3C, 0x11D3D), // NSM
    (0x11D3F, 0x11D41), // NSM
    (0x11D43, 0x11D43), // NSM
    (0x11D47, 0x11D47), // NSM
    // Gunjala Gondi
    (0x11D90, 0x11D91), // NSM
    (0x11D95, 0x11D95), // NSM
    // Makasar
    (0x11EF3, 0x11EF4), // NSM
    // Kawi
    (0x11F00, 0x11F01), // NSM
    (0x11F36, 0x11F3A), // NSM
    (0x11F40, 0x11F40), // NSM
    // Gurung Khema
    (0x1611E, 0x16129), // NSM
    (0x1612D, 0x1612E), // NSM
    // Miao
    (0x16F4F, 0x16F4F), // NSM
    (0x16F8F, 0x16F92), // NSM
    // Duployan
    (0x1BC9E, 0x1BC9E), // NSM
    // Glagolitic Supplement
    (0x1E000, 0x1E006), // NSM
    (0x1E008, 0x1E018), // NSM
    (0x1E01B, 0x1E021), // NSM
    (0x1E023, 0x1E024), // NSM
    (0x1E026, 0x1E02A), // NSM
    // Cyrillic Extended-D
    (0x1E08F, 0x1E08F), // NSM
    // Tai Yo
    (0x1E6E3, 0x1E6E3), // NSM
    (0x1E6E6, 0x1E6E6), // NSM
    (0x1E6EE, 0x1E6EF), // NSM
    (0x1E6F5, 0x1E6F5), // NSM
];
// Code points where the real `Bidi_Class` (per `DerivedBidiClass.txt`'s
// explicit, non-`@missing` per-code-point entries, Unicode 17.0.0) is `L`,
// but `Alphabetic=Yes` does not hold (`char::is_alphabetic() == false`) —
// see this function's doc, "`is_alphabetic()` fallback" section, case 2,
// for why this table exists and its scope. Restricted to entries whose
// `General_Category` (`DerivedGeneralCategory.txt`, same Unicode version)
// is `Nd` (decimal digit), one of the punctuation categories
// `Pc`/`Pd`/`Pe`/`Pf`/`Pi`/`Po`/`Ps`, or `Mc` (spacing combining mark) —
// plus a single `Mn` (non-spacing mark) entry that is functionally
// identical to the `Mc` ones (noted at its own site below) — see case 2's
// doc for why each of these shapes is included and what is deliberately
// left out. None of these ranges overlaps `AL_RANGES`/`R_RANGES` above
// (verified against this crate's actual `AL_RANGES`/`R_RANGES` tables at
// derivation time); none is `Alphabetic=Yes` under this workspace's
// pinned `char::is_alphabetic` (verified at derivation time) — except
// the three Sharada Vowel Signs Supplement entries noted at their own
// site below, which are `Alphabetic=Yes` under UCD 17.0.0 itself and
// need an entry here for exactly that reason (see their comment). Those
// three are individually-fixed instances of a much larger toolchain-lag
// class this table does not attempt to cover in full — see this
// function's doc, "`is_alphabetic()` fallback" section, case 2's second
// paragraph, for the fuller picture and why the rest is out of scope
// here. None overlaps `NON_STRONG_ALPHABETIC_RANGES` above — true by
// construction for every entry except those same three, since every
// other entry there has `Alphabetic=Yes` and every other entry here
// does not, while the three Sharada entries are instead checked
// directly against that table's actual code points (its `NSM` entries
// for the same Unicode block are 0x11B60, 0x11B62..0x11B64, 0x11B66 —
// disjoint from 0x11B61/0x11B65/0x11B67 here). Also checked directly at
// derivation time for every entry (range-pair overlap, not just
// individual code points, since both tables store ranges). So every
// entry here really would fall through to the final `None` below absent
// this check.
const STRONG_L_NON_ALPHABETIC_RANGES: &[(u32, u32)] = &[
    // Armenian
    (0x055A, 0x055F), // Po
    (0x0589, 0x0589), // Po
    // Devanagari
    (0x0964, 0x0965), // Po
    (0x0966, 0x096F), // Nd
    (0x0970, 0x0970), // Po
    // Bengali
    (0x09E6, 0x09EF), // Nd
    (0x09FD, 0x09FD), // Po
    // Gurmukhi
    (0x0A66, 0x0A6F), // Nd
    (0x0A76, 0x0A76), // Po
    // Gujarati
    (0x0AE6, 0x0AEF), // Nd
    (0x0AF0, 0x0AF0), // Po
    // Oriya
    (0x0B66, 0x0B6F), // Nd
    // Tamil
    (0x0BE6, 0x0BEF), // Nd
    // Telugu
    (0x0C66, 0x0C6F), // Nd
    (0x0C77, 0x0C77), // Po
    // Kannada
    (0x0C84, 0x0C84), // Po
    (0x0CE6, 0x0CEF), // Nd
    // Malayalam
    (0x0D66, 0x0D6F), // Nd
    // Sinhala
    (0x0DE6, 0x0DEF), // Nd
    (0x0DF4, 0x0DF4), // Po
    // Thai
    (0x0E4F, 0x0E4F), // Po
    (0x0E50, 0x0E59), // Nd
    (0x0E5A, 0x0E5B), // Po
    // Lao
    (0x0ED0, 0x0ED9), // Nd
    // Tibetan
    (0x0F04, 0x0F12), // Po
    (0x0F14, 0x0F14), // Po
    (0x0F20, 0x0F29), // Nd
    (0x0F85, 0x0F85), // Po
    (0x0FD0, 0x0FD4), // Po
    (0x0FD9, 0x0FDA), // Po
    // Myanmar
    (0x1040, 0x1049), // Nd
    (0x104A, 0x104F), // Po
    (0x1090, 0x1099), // Nd
    // Georgian
    (0x10FB, 0x10FB), // Po
    // Ethiopic
    (0x1360, 0x1368), // Po
    // Unified Canadian Aboriginal Syllabics
    (0x166E, 0x166E), // Po
    // Runic
    (0x16EB, 0x16ED), // Po
    // Hanunoo
    (0x1735, 0x1736), // Po
    // Khmer
    (0x17D4, 0x17D6), // Po
    (0x17D8, 0x17DA), // Po
    (0x17E0, 0x17E9), // Nd
    // Mongolian
    (0x1810, 0x1819), // Nd
    // Limbu
    (0x1946, 0x194F), // Nd
    // New Tai Lue
    (0x19D0, 0x19D9), // Nd
    // Buginese
    (0x1A1E, 0x1A1F), // Po
    // Tai Tham
    (0x1A80, 0x1A89), // Nd
    (0x1A90, 0x1A99), // Nd
    (0x1AA0, 0x1AA6), // Po
    (0x1AA8, 0x1AAD), // Po
    // Balinese
    (0x1B4E, 0x1B4F), // Po
    (0x1B50, 0x1B59), // Nd
    (0x1B5A, 0x1B60), // Po
    (0x1B7D, 0x1B7F), // Po
    // Sundanese
    (0x1BB0, 0x1BB9), // Nd
    // Batak
    (0x1BFC, 0x1BFF), // Po
    // Lepcha
    (0x1C3B, 0x1C3F), // Po
    (0x1C40, 0x1C49), // Nd
    // Ol Chiki
    (0x1C50, 0x1C59), // Nd
    (0x1C7E, 0x1C7F), // Po
    // Sundanese Supplement
    (0x1CC0, 0x1CC7), // Po
    // Vedic Extensions
    (0x1CD3, 0x1CD3), // Po
    // Tifinagh
    (0x2D70, 0x2D70), // Po
    // Lisu
    (0xA4FE, 0xA4FF), // Po
    // Vai
    (0xA620, 0xA629), // Nd
    // Bamum
    (0xA6F2, 0xA6F7), // Po
    // Saurashtra
    (0xA8CE, 0xA8CF), // Po
    (0xA8D0, 0xA8D9), // Nd
    // Devanagari Extended
    (0xA8F8, 0xA8FA), // Po
    (0xA8FC, 0xA8FC), // Po
    // Kayah Li
    (0xA900, 0xA909), // Nd
    (0xA92E, 0xA92F), // Po
    // Rejang
    (0xA95F, 0xA95F), // Po
    // Javanese
    (0xA9C1, 0xA9CD), // Po
    (0xA9D0, 0xA9D9), // Nd
    (0xA9DE, 0xA9DF), // Po
    // Myanmar Extended-B
    (0xA9F0, 0xA9F9), // Nd
    // Cham
    (0xAA50, 0xAA59), // Nd
    (0xAA5C, 0xAA5F), // Po
    // Tai Viet
    (0xAADE, 0xAADF), // Po
    // Meetei Mayek Extensions
    (0xAAF0, 0xAAF1), // Po
    // Meetei Mayek
    (0xABEB, 0xABEB), // Po
    (0xABF0, 0xABF9), // Nd
    // Aegean Numbers
    (0x10100, 0x10100), // Po
    (0x10102, 0x10102), // Po
    // Ugaritic
    (0x1039F, 0x1039F), // Po
    // Old Persian
    (0x103D0, 0x103D0), // Po
    // Osmanya
    (0x104A0, 0x104A9), // Nd
    // Caucasian Albanian
    (0x1056F, 0x1056F), // Po
    // Brahmi
    (0x11047, 0x1104D), // Po
    (0x11066, 0x1106F), // Nd
    // Kaithi
    (0x110BB, 0x110BC), // Po
    (0x110BE, 0x110C1), // Po
    // Sora Sompeng
    (0x110F0, 0x110F9), // Nd
    // Chakma
    (0x11136, 0x1113F), // Nd
    (0x11140, 0x11143), // Po
    // Mahajani
    (0x11174, 0x11175), // Po
    // Sharada
    (0x111C5, 0x111C8), // Po
    (0x111CD, 0x111CD), // Po
    (0x111D0, 0x111D9), // Nd
    (0x111DB, 0x111DB), // Po
    (0x111DD, 0x111DF), // Po
    // Khojki
    (0x11238, 0x1123D), // Po
    // Multani
    (0x112A9, 0x112A9), // Po
    // Khudawadi
    (0x112F0, 0x112F9), // Nd
    // Tulu-Tigalari
    (0x113D4, 0x113D5), // Po
    (0x113D7, 0x113D8), // Po
    // Newa
    (0x1144B, 0x1144F), // Po
    (0x11450, 0x11459), // Nd
    (0x1145A, 0x1145B), // Po
    (0x1145D, 0x1145D), // Po
    // Tirhuta
    (0x114C6, 0x114C6), // Po
    (0x114D0, 0x114D9), // Nd
    // Siddham
    (0x115C1, 0x115D7), // Po
    // Modi
    (0x11641, 0x11643), // Po
    (0x11650, 0x11659), // Nd
    // Takri
    (0x116B9, 0x116B9), // Po
    (0x116C0, 0x116C9), // Nd
    // Myanmar Extended-C
    (0x116D0, 0x116E3), // Nd
    // Ahom
    (0x11730, 0x11739), // Nd
    (0x1173C, 0x1173E), // Po
    // Dogra
    (0x1183B, 0x1183B), // Po
    // Warang Citi
    (0x118E0, 0x118E9), // Nd
    // Dives Akuru
    (0x11944, 0x11946), // Po
    (0x11950, 0x11959), // Nd
    // Nandinagari
    (0x119E2, 0x119E2), // Po
    // Zanabazar Square
    (0x11A3F, 0x11A46), // Po
    // Soyombo
    (0x11A9A, 0x11A9C), // Po
    (0x11A9E, 0x11AA2), // Po
    // Devanagari Extended-A
    (0x11B00, 0x11B09), // Po
    // Sunuwar
    (0x11BE1, 0x11BE1), // Po
    (0x11BF0, 0x11BF9), // Nd
    // Bhaiksuki
    (0x11C41, 0x11C45), // Po
    (0x11C50, 0x11C59), // Nd
    // Marchen
    (0x11C70, 0x11C71), // Po
    // Masaram Gondi
    (0x11D50, 0x11D59), // Nd
    // Gunjala Gondi
    (0x11DA0, 0x11DA9), // Nd
    // Tolong Siki
    (0x11DE0, 0x11DE9), // Nd
    // Makasar
    (0x11EF7, 0x11EF8), // Po
    // Kawi
    (0x11F43, 0x11F4F), // Po
    (0x11F50, 0x11F59), // Nd
    // Tamil Supplement
    (0x11FFF, 0x11FFF), // Po
    // Cuneiform Numbers and Punctuation
    (0x12470, 0x12474), // Po
    // Cypro-Minoan
    (0x12FF1, 0x12FF2), // Po
    // Gurung Khema
    (0x16130, 0x16139), // Nd
    // Mro
    (0x16A60, 0x16A69), // Nd
    (0x16A6E, 0x16A6F), // Po
    // Tangsa
    (0x16AC0, 0x16AC9), // Nd
    // Bassa Vah
    (0x16AF5, 0x16AF5), // Po
    // Pahawh Hmong
    (0x16B37, 0x16B3B), // Po
    (0x16B44, 0x16B44), // Po
    (0x16B50, 0x16B59), // Nd
    // Kirat Rai
    (0x16D6D, 0x16D6F), // Po
    (0x16D70, 0x16D79), // Nd
    // Medefaidrin
    (0x16E97, 0x16E9A), // Po
    // Duployan
    (0x1BC9F, 0x1BC9F), // Po
    // Sutton SignWriting
    (0x1DA87, 0x1DA8B), // Po
    // Nyiakeng Puachue Hmong
    (0x1E140, 0x1E149), // Nd
    // Wancho
    (0x1E2F0, 0x1E2F9), // Nd
    // Nag Mundari
    (0x1E4F0, 0x1E4F9), // Nd
    // Ol Onal
    (0x1E5F1, 0x1E5FA), // Nd
    (0x1E5FF, 0x1E5FF), // Po
    // The entries above are all `Nd`/punctuation. The entries below are
    // `Mc` (spacing combining mark), plus one `Mn` entry noted at its
    // own site — see this function's doc, case 2, for why these are
    // included here even though the surrounding doc text otherwise
    // talks about "named-script digits and punctuation": most `Mc`
    // code points with an explicit `Bidi_Class=L` entry are vowel signs
    // that are already `Alphabetic=Yes` and so already resolve
    // correctly through the `is_alphabetic()` branch above without
    // needing a table entry at all; these are the residual — viramas,
    // tone marks, and similar combining marks that Unicode does not
    // consider `Alphabetic` (a virama suppresses a vowel rather than
    // representing one) despite carrying `Bidi_Class=L` themselves.
    // Tibetan
    (0x0F3E, 0x0F3F), // Mc
    // Tagalog
    (0x1715, 0x1715), // Mc
    // Hanunoo
    (0x1734, 0x1734), // Mc
    // Balinese
    (0x1B44, 0x1B44), // Mc
    // Sundanese
    (0x1BAA, 0x1BAA), // Mc
    // Batak
    (0x1BF2, 0x1BF3), // Mc
    // Vedic Extensions
    (0x1CE1, 0x1CE1), // Mc
    (0x1CF7, 0x1CF7), // Mc
    // CJK Symbols and Punctuation (Hangul tone marks)
    (0x302E, 0x302F), // Mc
    // Rejang
    (0xA953, 0xA953), // Mc
    // Javanese
    (0xA9C0, 0xA9C0), // Mc
    // Meetei Mayek
    (0xABEC, 0xABEC), // Mc
    // Sharada
    (0x111C0, 0x111C0), // Mc
    // Khojki
    (0x11235, 0x11235), // Mc
    // Grantha
    (0x1134D, 0x1134D), // Mc
    // Tulu-Tigalari
    (0x113CF, 0x113CF), // Mc
    // Takri
    (0x116B6, 0x116B6), // Mc
    // Dives Akuru
    (0x1193D, 0x1193D), // Mc
    // Sharada Vowel Signs Supplement (new in Unicode 17.0.0). Excluded
    // from this workspace's pinned rustc 1.89.0 `char::is_alphabetic()`
    // tables, which predate this block's assignment, even though UCD
    // 17.0.0 marks these `Alphabetic=Yes` like the other vowel signs in
    // this script — so unlike most `Mc` vowel signs, these three still
    // need an explicit entry here rather than relying on the
    // `is_alphabetic()` branch.
    (0x11B61, 0x11B61), // Mc
    (0x11B65, 0x11B65), // Mc
    (0x11B67, 0x11B67), // Mc
    // Bhaiksuki (this one virama is `Mn`, not `Mc`, unlike its
    // counterparts above — same role, different General_Category)
    (0x11C3F, 0x11C3F), // Mn
    // Kawi
    (0x11F41, 0x11F41), // Mc
    // Musical Symbols
    (0x1D165, 0x1D166), // Mc
    (0x1D16D, 0x1D172), // Mc
];

/// Classifies a code point's Unicode Bidi_Class (Unicode Standard Annex #9,
/// the "\[BIDI\]" reference [`text_node_first_strong_direction`]'s HTML LS
/// quote cites) into one of the 3 *strong* types this crate's simplified
/// auto-directionality scan needs — `L`, `Al`, or `R` — or `None` for every
/// other Bidi_Class (the weak and neutral types: European/Arabic numbers,
/// separators, terminators, whitespace, neutral punctuation, combining
/// marks, controls, …), which is exactly the "not strong, keep scanning"
/// outcome [`text_node_first_strong_direction`]'s caller needs for those.
///
/// # Scope: default-block ranges only, not a full per-code-point Bidi_Class table
///
/// The Unicode Character Database's actual `Bidi_Class` property is a
/// complete per-code-point assignment (`DerivedBidiClass.txt`,
/// <https://www.unicode.org/Public/UCD/latest/ucd/extracted/DerivedBidiClass.txt>)
/// — implementing that in full is a multi-thousand-range data table on its
/// own, comparable in scope to the RFC4647/BCP47 canonicalization table
/// [`language_range_matches`]'s doc similarly declines to bring in for a
/// different feature. This function instead hard-codes exactly the ranges
/// that same UCD file's own "`@missing`" comments give as the **default**
/// Bidi_Class for every script block the Unicode Standard reserves for
/// right-to-left use (`AL_RANGES` / `R_RANGES` below, transcribed directly
/// from those `@missing` lines, and guarded against the embedded
/// non-default code points inside them by `NON_STRONG_WITHIN_AL_R_RANGES`,
/// see below) — this covers every script block Unicode reserves by default
/// for Arabic-derived (`AL`) or other right-to-left (`R`) use, not just
/// "the major ones". Any code point outside all of those ranges falls back
/// to [`char::is_alphabetic`] (guarded by `NON_STRONG_ALPHABETIC_RANGES`,
/// see below) for `L` (covers Latin, Greek,
/// Cyrillic, CJK, Hangul, Devanagari, and effectively every other
/// left-to-right alphabetic script), then to `STRONG_L_NON_ALPHABETIC_RANGES`
/// (see below) for a further set of code points whose real `Bidi_Class` is
/// `L` despite not being alphabetic (non-Latin decimal digits,
/// script-specific punctuation, and viramas/tone marks that Unicode does
/// not tag `Alphabetic`), or `None` for everything else — which matches
/// the real `Bidi_Class` table for most of those too (their actual classes
/// are typically weak/neutral types like EN, AN, CS, ON, WS, NSM). This is
/// not exhaustive — case 2 of the `is_alphabetic()` fallback section below
/// documents two further residuals left deliberately unguarded beyond
/// `STRONG_L_NON_ALPHABETIC_RANGES`'s fixed `Nd`/punctuation/`Mc`
/// categories: a version-drift-driven one, where a Unicode revision newer
/// than what the pinned toolchain's `is_alphabetic()` was built against
/// assigns real `Bidi_Class=L` to code points this function does not yet
/// special-case (currently on the order of thousands, of which only three
/// are individually listed), and a smaller, toolchain-independent
/// structural one (`So`/`No` symbols and numbers) — see case 2 for both
/// residuals' shape and why each is left unguarded. Three specific code
/// points *are* handled explicitly before
/// reaching any of this, cheaply, without a table: U+200E LEFT-TO-RIGHT
/// MARK and U+200F RIGHT-TO-LEFT MARK
/// (`Cf`, General Punctuation block) and U+061C ARABIC LETTER MARK (`Cf`,
/// Arabic block) all have an explicit (non-`@missing`)
/// `DerivedBidiClass.txt` entry of `L`, `R`, and `AL` respectively — none
/// is alphabetic, so none would resolve correctly through the
/// `is_alphabetic`-based fallback above. U+061C already falls
/// inside `AL_RANGES`'s main Arabic range (U+0600..U+07BF) and needs no
/// special handling; U+200E and U+200F fall inside neither `AL_RANGES` nor
/// `R_RANGES`, so this function checks for those two explicitly, before
/// either range table, at the top of its body.
///
/// A known imprecision from using whole-block ranges rather than the real
/// per-code-point table: a handful of code points *inside* the
/// `AL_RANGES`/`R_RANGES` below (e.g. Arabic-Indic digits, Hebrew
/// punctuation and accents) have a real `Bidi_Class` that is actually a
/// weak/neutral type, not `AL`/`R`. Over-classifying such a code point as
/// strong `AL`/`R` can make the scan stop *and resolve* on it before ever
/// reaching the text's true first strong character, flipping the result
/// outright — not a harmless "one code point early" scan offset with the
/// same eventual outcome. Example: U+0664 ARABIC-INDIC DIGIT FOUR falls
/// inside `AL_RANGES`'s main Arabic span but its real `Bidi_Class` is `AN`
/// (Arabic Number, weak); in `"\u{0664}H"` the spec skips it and resolves
/// on the following Latin `H` (`L`) → `ltr`, whereas over-classifying the
/// digit as strong `AL` would wrongly stop the scan there and resolve
/// `rtl`. `NON_STRONG_WITHIN_AL_R_RANGES` below closes this gap: every
/// explicit (non-`@missing`) `DerivedBidiClass.txt` entry whose range
/// falls inside an `AL_RANGES`/`R_RANGES` span and whose class is not that
/// span's own `AL`/`R` default (74 entries for Unicode 17.0.0 — none of
/// them `L`, only other weak/neutral types: `NSM`, `ON`, `AN`, `EN`, `ET`,
/// `ES`, `CS`, `BN`) — the same "guard the default-block table with the
/// exceptions extracted from the same source file" shape
/// `NON_STRONG_ALPHABETIC_RANGES` below uses for the `is_alphabetic()`
/// fallback. This function checks it before either range table and treats
/// a match as non-strong (`None`), so `AL_RANGES`/`R_RANGES` membership
/// alone no longer over-classifies these code points as strong. Unlike
/// `NON_STRONG_ALPHABETIC_RANGES` (which guards `char::is_alphabetic`, a
/// property computed by a separate, independently-versioned Rust/Unicode
/// table — see its own version-drift caveat below), `AL_RANGES`/`R_RANGES`
/// and this table are both transcribed from the same `DerivedBidiClass.txt`
/// snapshot (Unicode 17.0.0), so there is no comparable cross-source
/// version skew to caveat here.
///
/// # `is_alphabetic()` fallback: two gaps against the real `Bidi_Class`
///
/// Outside `AL_RANGES`/`R_RANGES`, `L` is derived from
/// [`char::is_alphabetic`] — i.e. from Unicode's `Alphabetic` property
/// (`DerivedCoreProperties.txt`), a *different* property than `Bidi_Class`.
/// The two properties agree for the overwhelming majority of code points,
/// but diverge in two ways that can each flip the resolved direction
/// outright:
///
/// 1. **False `L`**: some code points are `Alphabetic=Yes` but their real
///    `Bidi_Class` is not `L` at all — mostly `Mn`/`Mc` combining marks
///    classified `NSM`, plus a smaller set of `Lm` spacing modifier
///    letters and one `Nl` numeral-symbol block (Ancient Greek Numbers,
///    U+10140..U+10174), all classified `ON`. Left unguarded,
///    `is_alphabetic()` would misclassify these as strong `L` and stop the
///    scan right there, even though the text's real first strong character
///    is later. Example: U+0941 DEVANAGARI VOWEL SIGN U is
///    `Other_Alphabetic` (`is_alphabetic() == true`) but `Bidi_Class=NSM`
///    (non-strong); in `"\u{0941}שלום"` the spec skips it and resolves on
///    the following Hebrew (`R`) code point → `rtl`, whereas treating
///    U+0941 itself as strong `L` would wrongly stop the scan there and
///    resolve `ltr`. `NON_STRONG_ALPHABETIC_RANGES` below closes this gap:
///    for Unicode 17.0.0, it is the full set of code points with
///    `Alphabetic=Yes` and a `Bidi_Class` other than `L`, outside
///    `AL_RANGES`/`R_RANGES` (whose own over-classification exceptions are
///    the separate gap `NON_STRONG_WITHIN_AL_R_RANGES` above closes) —
///    derived by intersecting
///    `DerivedCoreProperties.txt`'s `Alphabetic` ranges against
///    `DerivedBidiClass.txt`'s explicit per-code-point entries, both at
///    that same Unicode version. `char::is_alphabetic` itself may track a
///    different Unicode revision than 17.0.0 (this crate does not pin
///    Rust's own Unicode table version); any code point added or
///    reclassified between that revision and 17.0.0 is not guaranteed to
///    be covered. This function consults the table before falling back to
///    `is_alphabetic`, and treats a match as non-strong (`None`) rather
///    than `L`.
///
/// 2. **False non-`L`**: the opposite gap — some code points have real
///    `Bidi_Class=L` but `is_alphabetic() == false`, mostly decimal digits
///    of non-Latin scripts (e.g. U+0966..U+096F DEVANAGARI DIGIT
///    ZERO..NINE) plus a smaller set of punctuation (e.g. U+055A..U+055F
///    Armenian punctuation). Left uncorrected, a genuinely-first `L`
///    character in this set would be skipped in favor of a later `AL`/`R`
///    character in the same text, resolving the wrong direction outright
///    — the same failure shape as case 1, just in the opposite direction.
///    `STRONG_L_NON_ALPHABETIC_RANGES` below closes the categorical part
///    of this gap: every explicit (non-`@missing`) `DerivedBidiClass.txt`
///    entry whose `Bidi_Class=L` and whose `General_Category`
///    (`DerivedGeneralCategory.txt`, same Unicode version) is `Nd`
///    (decimal digit), one of the punctuation categories
///    `Pc`/`Pd`/`Pe`/`Pf`/`Pi`/`Po`/`Ps`, or `Mc` (spacing combining mark,
///    plus one functionally-identical `Mn` entry) — 186 entries for
///    Unicode 17.0.0, exhaustive for exactly those categories plus the
///    three Sharada Vowel Signs Supplement code points below, which are
///    added for a different reason covered in the next paragraph, not
///    because a fourth category exists. The first two categories are
///    named-script digits and punctuation; `Mc` needs its own
///    explanation, since most `Mc` code points are vowel signs already
///    covered by `Alphabetic=Yes` — the ones actually needed here are
///    viramas, tone marks, and similar combining marks that Unicode does
///    not consider `Alphabetic` under any Unicode version, despite their
///    real `Bidi_Class=L`. (One member of this same case-2 category,
///    U+200E LEFT-TO-RIGHT MARK, is handled separately — not by this table but
///    by the explicit top-of-function check described above, since that code
///    point was already being singled out for U+200F's sake.)
///
///    A second, unrelated route into this same "real `L`,
///    `is_alphabetic() == false`" shape is version drift, mirroring case
///    1's caveat above: `char::is_alphabetic` may track a Unicode
///    revision older than the 17.0.0 this table's categories are derived
///    against, so a code point that Unicode 17.0.0 assigns both
///    `Bidi_Class=L` and `Alphabetic=Yes` can still fail
///    `is_alphabetic()` under an older-than-17.0.0 toolchain — even
///    though, by rights, it should need no table entry at all and be
///    caught by the plain `c.is_alphabetic()` branch above. The three
///    Sharada Vowel Signs Supplement code points below are exactly this:
///    `Mc`, `Alphabetic=Yes` under UCD 17.0.0, and so already inside this
///    table's `Nd`/punctuation/`Mc` category filter, but listed
///    individually because they are the three code points this table's
///    own derivation verified as affected. They are not the only code
///    points this mechanism affects. Checking this workspace's pinned rustc 1.89.0
///    (`rust-toolchain.toml`) against the same `Bidi_Class=L` ∧
///    `Alphabetic=Yes` (Unicode 17.0.0) intersection finds 4620 such code
///    points failing `char::is_alphabetic()`, not 3 — dominated by CJK
///    Unified Ideographs Extension J alone (4298 code points,
///    U+323B0..U+33479), plus Tangut Ideographic Components and a handful
///    of other letter blocks Unicode 17.0.0 newly assigned (Tai Yo, Beria
///    Erfe, Tolong Siki, Latin Extended-D). Checked directly, not just
///    assumed: of the 4620, exactly 3 are `General_Category` `Mc` — the
///    same three Sharada entries already listed — and none is `Mn`,
///    `Nd`, or one of the punctuation categories; the remaining 4617 are
///    `Lo` (4555), `Lu` (28), `Ll` (26), `Lm` (5), or `Nl` (3) letters
///    and letter-numbers, entirely outside this table's
///    `Nd`/punctuation/`Mc` category filter. So covering them here would
///    mean dropping that filter and turning this into a general
///    "toolchain hasn't caught up to Unicode 17.0.0 yet" table with a
///    completely different, much larger and version-churn-prone scope,
///    not an extension of the current one. Hardcoding thousands of
///    entries for that — one CJK block alone accounts for the
///    overwhelming majority — would be exactly the kind
///    of exhaustive per-code-point enumeration this function's tables
///    otherwise avoid, and it would go silently stale (redundant, not
///    merely unneeded) the moment the pinned toolchain is updated past
///    17.0.0, since `char::is_alphabetic` would then cover these code
///    points on its own. Closing this class of gap for good would mean
///    either updating the pinned rustc toolchain or deriving the
///    `Alphabetic` check directly from UCD data instead of
///    `char::is_alphabetic` — both out of scope for this table. Left as
///    a known, open-ended limitation whose size tracks how far the
///    pinned toolchain lags the newest Unicode data, unlike the closed,
///    version-independent `Nd`/punctuation/`Mc` categories above.
///
///    Separately from version drift, a residual is deliberately left
///    uncovered regardless of toolchain version: explicit `Bidi_Class=L`
///    code points outside `Nd`/punctuation/`Mc` whose real
///    `General_Category` is `So`/`No` (symbols and other numbers). This
///    category is not a clean "symbols, not script text" line the way it
///    might sound — Ethiopic's entire digit series (U+1369..U+137C
///    ETHIOPIC DIGIT ONE..ETHIOPIC NUMBER TEN THOUSAND) is `No`, not `Nd`,
///    because Ethiopic historically has no zero and its "digits" are
///    additive numeral signs rather than positional digits; smaller
///    numeral-adjacent `No` sets exist for Tamil, Bengali, Oriya,
///    Malayalam, Sinhala, and Tibetan half-integers alongside the Braille
///    patterns, circled/parenthesized digits, and squared/circled CJK
///    compatibility symbols that are genuinely symbol-like. Also left out:
///    a handful of `Cf` format characters and `Sk` modifier symbols with
///    real `Bidi_Class=L` that are not `Alphabetic=Yes` (e.g. U+110BD
///    KAITHI NUMBER SIGN / U+110CD KAITHI NUMBER SIGN ABOVE,
///    U+A789..U+A78A MODIFIER LETTER COLON..MODIFIER LETTER SHORT EQUALS
///    SIGN), and the three Private Use Area blocks (`Co`, two of them
///    65534 code points wide, one — the BMP Private Use Area — 6400),
///    which also carry an explicit `Bidi_Class=L` entry each. All of these
///    are excluded by the same category check as the rest of the residual,
///    not specially cased. Unlike the `Mc` viramas this table does cover,
///    the `So`/`No` residual has no single derivation rule that reliably
///    separates "numeral sign genuinely likely to open real text" from
///    "compatibility symbol essentially never seen as the first character
///    of a sentence" — chasing it further would mean enumerating the
///    residual case by case rather than by a `General_Category` rule,
///    which this function's tables otherwise avoid doing. Left as a known,
///    narrowly-scoped gap: it requires one of these residual code points
///    to itself be the text's true first strong character ahead of an
///    unrelated `AL`/`R` character elsewhere in the same string.
///    (Separately, and much larger again: `DerivedBidiClass.txt` also
///    carries a single `@missing: 0000..10FFFF; Left_To_Right` line — the
///    default `Bidi_Class` for every *unassigned* code point in the entire
///    codespace, dwarfing every table on this page combined. That default
///    plays no part in this table's derivation: like
///    `NON_STRONG_ALPHABETIC_RANGES` above, this table only draws from
///    explicit, non-`@missing` per-code-point entries — unlike
///    `AL_RANGES`/`R_RANGES`, which do transcribe `@missing` lines, but
///    only the narrower, per-script-block ones, never this codespace-wide
///    one.)
fn strong_bidi_type(c: char) -> Option<StrongBidiType> {
    let cp = c as u32;
    // U+200E LEFT-TO-RIGHT MARK and U+200F RIGHT-TO-LEFT MARK are explicit
    // per-code-point `DerivedBidiClass.txt` entries (`Cf` General
    // Punctuation, `; L` and `; R` respectively) outside every range in
    // `AL_RANGES`/`R_RANGES` below — see this function's `# Scope` doc
    // section for the full 3-code-point picture including U+061C ARABIC
    // LETTER MARK (which needs no arm here: already `AL_RANGES`-covered).
    // Checked first, before either range table, so neither falls through to
    // the `is_alphabetic` branch below — both are format characters
    // (`is_alphabetic() == false`), which would otherwise return `None` for
    // both instead of their real strong type.
    match cp {
        0x200E => return Some(StrongBidiType::L),
        0x200F => return Some(StrongBidiType::R),
        _ => {}
    }
    if NON_STRONG_WITHIN_AL_R_RANGES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&cp))
    {
        return None;
    }
    if AL_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
        return Some(StrongBidiType::Al);
    }
    if R_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
        return Some(StrongBidiType::R);
    }
    if NON_STRONG_ALPHABETIC_RANGES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&cp))
    {
        return None;
    }
    if c.is_alphabetic() {
        return Some(StrongBidiType::L);
    }
    if STRONG_L_NON_ALPHABETIC_RANGES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&cp))
    {
        return Some(StrongBidiType::L);
    }
    None
}

/// `Component::AttributeInNoNamespace`'s `ParsedCaseSensitivity` (spec-only,
/// "language depends on this" placeholder for the
/// `AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument` case) を、実際に
/// `AttrSelectorOperator::eval_str` へ渡せる `CaseSensitivity` へ解決する。
///
/// upstream `selectors::matching::to_unconditional_case_sensitivity` と同じ
/// 3-way 分岐を model 化しているが、その関数は tree-walk 込みの重い
/// `selectors::Element` trait を要求するため呼べない (raikiri の
/// `StyleElement` は single-element matching 用の縮小 trait —
/// `wall/traits` を跨がない private helper として
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
/// この関数へ渡す配線が必要になる。
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

/// `<img width>` / `<img height>` の HTML presentational-hint 昇格。
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
///   spec 段落の後続文が他要素にも同じ mapping を適用するが、本関数の
///   scope narrowing は `img` のみに限定している (最小実装、将来の拡張の
///   土台という位置づけ)。
///
/// # Cascade origin (`Origin::AuthorPresentationalHint` へ retag 済み、
/// spec text 再確認済み)
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
/// `(User, false) => 1` より上、`(Author, false) => 3` より下に置く
/// ([`Origin::User`] 挿入後の値 — [`cascade_rank`] doc
/// 参照)。この rank 差は `beats` の tuple compare `(rank, specificity, source_order)` の
/// **第一要素**なので、真の UA-origin rule には specificity/source_order を
/// 問わず常に勝ち、real author-origin 宣言 (stylesheet rule でも inline
/// style でも) には specificity/source_order を問わず常に負ける — かつて
/// (retag 前、旧版は hint も real 宣言も同じ `Origin::Author`
/// に tag していた) は後者の保証を「hint の specificity を 0 に固定し、
/// real 宣言が zero-specificity かつ stylesheet 先頭 rule の場合に限り
/// 発生する exact tie を push 順序 (hint を先に push) で決着させる」という
/// 同一 origin 内 tie-break に依存していた — 3rd tier 導入によりその依存は
/// 解消され、origin rank だけで無条件に決着する。[`Origin::User`] の挿入は
/// この結論を変えない — hint の rank は挿入後も
/// 依然として real `Author` rank と等しくなることが無い (`AuthorPresentationalHint`
/// と `Author` は常に隣接する別 rank 値のまま、[`cascade_rank`] doc の rank
/// 表参照) ため、[`collect_cascaded`] が今も stylesheet rule matching /
/// inline style より先にこの関数を push する呼び出し順は残っているが、
/// 上記の通りもう correctness の必要条件ではない (無害な残置、re-verify 済み)。
///
/// テスト
/// `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
/// はこの「specificity を問わず real author 宣言が勝つ」性質を、かつては
/// exact-tie 経由で、今は origin rank 差で直接 exercise する ([`Origin::User`]
/// 挿入後も rank 差の大小関係は変わらないため、この test は無変更で pin
/// し続ける)。
///
/// `Origin::User` no-producer 残差の解消: raikiri-style 内の
/// [`Origin::User`] variant 自体を追加した時点 ([`cascade_rank`] の 4-tier
/// 化) では、consumer が渡す `extra_stylesheets` を実際に [`Origin::User`]
/// へ route する producer がまだ無く、今も `StylesheetKind::Author` 経由で
/// [`Origin::Author`] として届いていた。spec の完全な順序では hint は
/// 「user origin より強い」はずだが、当時の実装は user stylesheet 宣言を
/// hint より強い [`Origin::Author`] rank に一律 fold していた — user
/// stylesheet が img の width/height を上書きできるという結果自体は spec と
/// 一致していたが (`Author` rank は hint より常に上)、独立した User origin
/// へ実際に route されていない分、モデルの精度としては不完全だった。
///
/// raikiri-traits 側の `StylesheetKind` に独立
/// `User` variant を追加し raikiri-html で retag、umbrella 側の
/// `stylesheet_kind_to_origin` を拡張する genuine multi-crate diff
/// (raikiri-style 単体では完結しない) が着地し、この残差は解消された —
/// `extra_stylesheets` は今は実際に [`Origin::User`] へ route される。**normal
/// 宣言同士なら** user stylesheet の宣言はもう [`Origin::Author`] rank に
/// fold されず、spec 通り hint ([`Origin::AuthorPresentationalHint`]、normal
/// rank 2) より弱い ([`Origin::User`] normal rank 1) — つまり `<img width>`
/// hint は今や normal な `extra_stylesheets` 由来の宣言に specificity を
/// 問わず勝つ (real author-origin 宣言、たとえば in-document `<style>`、には
/// normal 同士なら今も負ける — umbrella crate の `build_cascaded` doc の
/// "DOM `<style>` vs `extra_stylesheets`" 節参照)。ただし
/// `extra_stylesheets` 側が `!important` を持つ場合はこの勝敗も反転する:
/// [`Origin::User`] の important rank (6) は hint の (常に normal で push
/// される、[`cascade_rank`] doc 参照) rank (2) より高いため、`!important`
/// 付きの `extra_stylesheets` 宣言は hint に specificity を問わず勝つ。この
/// normal-tier の振る舞いの umbrella 越し end-to-end pin は
/// `crates/raikiri/tests/build_cascaded.rs`'s
/// `img_width_presentational_hint_beats_extra_stylesheets_user_origin_via_umbrella`
/// 参照。
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
    // (`tag.eq_ignore_ascii_case("template") && elem.namespace_uri().is_none()`)
    // and the same principle applies to attribute-selector matching.
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

/// The "elements with default margins" HTML LS §15.3.9 "Margin collapsing
/// quirks"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>)
/// names by an explicit list, verbatim: blockquote, dir, dl, h1, h2, h3,
/// h4, h5, h6, listing, menu, ol, p, plaintext, pre, ul, xmp (17 elements).
///
/// `eq_ignore_ascii_case`, not plain `==` — matched against a *hardcoded
/// literal* set, not against another DOM-sourced tag name (unlike
/// [`sibling_position`]'s sibling-vs-sibling comparison, whose "html5ever
/// already normalises" reasoning only covers two DOM-sourced tag names
/// meeting each other). Nothing in [`StyleElement::tag_name`]'s own
/// contract requires lower-casing — the crate's own test-only `TestDoc`
/// mock explicitly does not lower-case — so this stays defensive the
/// same way the other same-file examples of
/// "DOM tag name vs. hardcoded literal" already are:
/// `elem.tag_name().eq_ignore_ascii_case("img")`
/// ([`push_img_dimension_hints`]) and
/// `tag.eq_ignore_ascii_case("template")` (`ruletree.rs`'s `<template>`
/// gate).
fn is_element_with_default_margins(tag_name: &str) -> bool {
    const NAMES: &[&str] = &[
        "blockquote",
        "dir",
        "dl",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "listing",
        "menu",
        "ol",
        "p",
        "plaintext",
        "pre",
        "ul",
        "xmp",
    ];
    NAMES.iter().any(|name| tag_name.eq_ignore_ascii_case(name))
}

/// HTML LS §15.3.9 defines a node as "substantial" if it is a text node
/// that is not [inter-element
/// whitespace](https://html.spec.whatwg.org/multipage/dom.html#inter-element-whitespace),
/// or if it is an element node.
///
/// "Inter-element whitespace" (HTML LS §3.2.5 "Content models", same URL
/// as above, verbatim): "Empty Text nodes and Text nodes consisting of
/// just \[...\] ASCII whitespace \[...\] are considered inter-element
/// whitespace" — "ASCII whitespace" itself links to Infra's
/// <https://infra.spec.whatwg.org/#ascii-whitespace> dfn there, the
/// 5-character set {tab, LF, FF, CR, space}. Same set this crate already
/// uses for HTML attribute-value tokenisation (`class_token_matches` in
/// [`crate::style_dom`]) and dimension-value leading-whitespace skipping
/// ([`parse_html_dimension_value`]).
///
/// This is deliberately a *different* character set from CSS Selectors
/// L4's `:empty` "document white space characters" ([`matches_empty`]
/// doc) — that set is {space, tab, segment break/LF} and excludes form
/// feed; HTML LS's "ASCII whitespace" includes form feed and carriage
/// return too. The two predicates ([`matches_empty`] for `:empty`, this
/// function's caller [`is_blank_element`] for HTML LS "blank") are
/// spec-distinct features and must not share one whitespace set even
/// though they look similar.
///
/// Comment / processing-instruction / document-fragment / document nodes
/// are none of "text node" or "element node", so they are never
/// substantial — matching how [`matches_empty`] treats the same kinds as
/// not affecting `:empty` (verbatim spec text quoted on that function:
/// "comments, processing instructions, and other nodes must not affect").
fn is_substantial_node<D: StyleDom>(dom: &D, id: StyleNodeId) -> bool {
    match dom.node(id) {
        Some(node) => match node.kind() {
            StyleNodeKind::Element => true,
            StyleNodeKind::Text => !node
                .text_content()
                .unwrap_or("")
                .chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')),
            StyleNodeKind::Comment
            | StyleNodeKind::ProcessingInstruction
            | StyleNodeKind::DocumentFragment
            | StyleNodeKind::Document => false,
        },
        // cov:ignore: `child_ids` only ever yields ids that `dom.node`
        // resolves (`StyleDom` trait doc: "child_ids(id) returns an empty
        // iterator for invalid id", implying ids it does yield are
        // valid) — same defensive posture as `matches_empty`'s own `None
        // => true` fallback. This function's only two call sites
        // (`is_blank_element`, `substantial_sibling_bounds`) always
        // derive `id` from `dom.child_ids`.
        None => false,
    }
}

/// HTML LS §15.3.9 defines an element as "blank" if it "contains no
/// substantial nodes"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>).
///
/// Only `elem_id`'s **direct** children need checking, not the full
/// descendant subtree: [`is_substantial_node`] already counts *any*
/// element node as substantial regardless of what that element itself
/// contains, so the moment `elem_id` has one element child anywhere in its
/// subtree, that child is itself either a substantial direct child, or
/// the child leading to it is — "does any direct child qualify" and "does
/// any descendant at any depth qualify" always agree for this particular
/// definition of substantial.
fn is_blank_element<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> bool {
    !dom.child_ids(elem_id)
        .any(|child_id| is_substantial_node(dom, child_id))
}

/// For `target_id` among `parent_id`'s direct children, whether it has no
/// substantial sibling **before** it and no substantial sibling **after**
/// it in document order (HTML LS §15.3.9's "has no substantial previous
/// siblings" / "has no substantial following siblings" conditions,
/// <https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>).
/// Single pass over `parent_id`'s children — same shape as
/// [`sibling_position`]'s combined start/end computation.
///
/// `target_id` not found among `parent_id`'s children never happens for
/// this function's only caller
/// ([`push_margin_collapsing_quirk_declarations`]): `parent_id` there is
/// always `ancestor_path.last()`, i.e. the real DOM parent
/// [`collect_cascaded`] walked through `dom.child_ids(parent_id)` to reach
/// `target_id` in the first place — the same ancestor-path invariant
/// [`sibling_position`]'s own callers rely on.
fn substantial_sibling_bounds<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    target_id: StyleNodeId,
) -> (bool, bool) {
    let mut no_substantial_before = true;
    let mut no_substantial_after = true;
    let mut seen_target = false;
    for child_id in dom.child_ids(parent_id) {
        if child_id == target_id {
            seen_target = true;
            continue;
        }
        if is_substantial_node(dom, child_id) {
            if seen_target {
                no_substantial_after = false;
            } else {
                no_substantial_before = false;
            }
        }
    }
    (no_substantial_before, no_substantial_after)
}

/// Margin-collapsing quirks (HTML LS §15.3.9 "Margin collapsing quirks",
/// <https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>
/// — fetched as raw spec HTML directly rather than through a summarizing
/// fetch, the same precaution [`matches_empty`]'s doc explains: this
/// section sits deep inside one very long single-page spec, where
/// summarized fetches have been observed to truncate before reaching the
/// relevant section).
///
/// Four verbatim rules, all gated on [`StyleQuirksMode::Quirks`] (the DOM
/// Standard's full "quirks mode", distinct from "limited-quirks mode" —
/// same distinction [`crate::style_dom::StyleQuirksMode`]'s own doc
/// draws):
///
/// 1. Any "element with default margins"
///    ([`is_element_with_default_margins`]) that is the child of a
///    `body`/`td`/`th` element and has no substantial previous siblings:
///    `margin-block-start` (this crate's `margin-top` — no writing-mode
///    support, so physical == logical unconditionally here, same posture
///    minimal.css's own `margin-block` substitution already takes)
///    zeroed.
/// 2. Same as 1, plus the element is "blank" ([`is_blank_element`]):
///    `margin-block-end` (`margin-bottom`) also zeroed.
/// 3. Any such element that is the child of a `td`/`th` element, has no
///    substantial following siblings, and is blank: `margin-block-start`
///    zeroed.
/// 4. Any `p` that is the child of a `td`/`th` element and has no
///    substantial following siblings: `margin-block-end` zeroed.
///
/// # Why this enters cascade as a candidate declaration rather than
/// overriding the computed value directly
///
/// The spec frames all four rules as "is expected to have a user-agent
/// level style sheet rule that sets \[...\] to zero" — i.e. a real UA
/// stylesheet declaration participating in normal cascade, not an
/// unconditional override. Concretely: real author CSS (any specificity,
/// `Origin::Author`) must still be able to give the element a nonzero
/// margin again; zeroing the resolved computed value after cascade
/// (bypassing origin/specificity entirely) would incorrectly clobber
/// that. So this function pushes an [`Origin::UserAgent`] candidate
/// declaration into the same flat candidate list [`collect_cascaded`]
/// already builds for this node from stylesheet rules and inline style,
/// and lets the normal [`pick_winners`]/[`beats`] machinery decide —
/// [`cascade_rank`] guarantees any `Origin::Author` declaration for the
/// same property outranks this regardless of specificity. See
/// [`MARGIN_COLLAPSING_QUIRK_SPECIFICITY`]'s own doc for why this
/// declaration's specificity still needs to be chosen carefully (to
/// out-rank *other* `Origin::UserAgent` declarations for the same
/// property, e.g. minimal.css's default-margin rule).
///
/// # Why the structural predicates can't be plain CSS selectors
///
/// "No substantial previous/following sibling" counts text-node content
/// (ignoring only inter-element whitespace), which CSS Selectors L4's
/// `:first-child`/`:last-child` do not — those ignore *all* non-element
/// siblings regardless of text content ([`sibling_position`] doc,
/// verbatim: "Standalone text and other non-element nodes are not counted
/// \[...\]"). A document like `<body>Hello<p>...</p></body>` has `<p>` as
/// CSS's `:first-child` (no earlier *element* sibling) but HTML LS denies
/// it "no substantial previous siblings" (the text "Hello" is
/// substantial) — a selector-based UA rule built on `:first-child` would
/// zero this `<p>`'s margin-block-start incorrectly. Hence the dedicated
/// structural predicates ([`is_substantial_node`],
/// [`substantial_sibling_bounds`], [`is_blank_element`]), computed
/// directly against the [`StyleDom`] tree rather than expressed as
/// selector components.
///
/// # HTML-namespace gate
///
/// Both `elem` and its `body`/`td`/`th` container are gated on
/// `namespace_uri().is_none()` — same posture
/// [`push_img_dimension_hints`]'s own doc explains for the same reason:
/// HTML LS's rendering rules (§15.2's own `@namespace
/// "http://www.w3.org/1999/xhtml";` scoping, which §15.3.9 falls under)
/// are scoped to the HTML namespace, and [`StyleElement`] is a generic
/// trait not tied to any one DOM/parser, so this stays defensive rather
/// than relying on "no foreign-namespace vocabulary defines an element
/// that happens to share one of these local names today".
fn push_margin_collapsing_quirk_declarations<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    elem: &impl StyleElement,
    ancestor_path: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    decls: &mut Vec<CascadedDecl>,
) {
    if quirks_mode != StyleQuirksMode::Quirks || elem.namespace_uri().is_some() {
        return;
    }
    let tag_name = elem.tag_name();
    if !is_element_with_default_margins(tag_name) {
        return;
    }
    let Some(&parent_id) = ancestor_path.last() else {
        return;
    };
    // cov:ignore: `parent_id` came from `ancestor_path`, which
    // `collect_cascaded` only ever pushes `Element`-kind ids onto (this
    // module's doc on `ancestor_path`), so `dom.node(parent_id)` resolving
    // to a node whose `as_element()` is `Some` is guaranteed by that same
    // invariant, not by anything local to this function. Defensive
    // fallback kept anyway rather than an unchecked index, same posture
    // as `sibling_position`'s own `dom.node(child_id)`/`as_element()`
    // guards over `child_ids`.
    let Some(parent_node) = dom.node(parent_id) else {
        return;
    };
    // cov:ignore: see the comment on the `dom.node(parent_id)` guard above
    // — same invariant covers this arm too.
    let Some(parent_elem) = parent_node.as_element() else {
        return;
    };
    if parent_elem.namespace_uri().is_some() {
        return;
    }
    let parent_tag = parent_elem.tag_name();
    let is_body = parent_tag.eq_ignore_ascii_case("body");
    let is_td_or_th =
        parent_tag.eq_ignore_ascii_case("td") || parent_tag.eq_ignore_ascii_case("th");
    if !is_body && !is_td_or_th {
        return;
    }

    let (no_substantial_before, no_substantial_after) =
        substantial_sibling_bounds(dom, parent_id, id);
    let is_blank = is_blank_element(dom, id);

    let mut zero_start = false;
    let mut zero_end = false;

    // Rules 1 + 2: body/td/th child, no substantial previous sibling.
    if no_substantial_before {
        zero_start = true;
        if is_blank {
            zero_end = true;
        }
    }
    // Rule 3: td/th child, no substantial following sibling, blank.
    if is_td_or_th && no_substantial_after && is_blank {
        zero_start = true;
    }
    // Rule 4: p child of td/th, no substantial following sibling.
    if is_td_or_th && tag_name.eq_ignore_ascii_case("p") && no_substantial_after {
        zero_end = true;
    }

    if zero_start {
        decls.push((
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(0.0))),
            false,
            Origin::UserAgent,
            MARGIN_COLLAPSING_QUIRK_SPECIFICITY,
            MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER,
        ));
    }
    if zero_end {
        decls.push((
            PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(0.0))),
            false,
            Origin::UserAgent,
            MARGIN_COLLAPSING_QUIRK_SPECIFICITY,
            MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER,
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
/// value 一般、非負整数だけでなく小数・percentage も受理) —
/// "非負整数" という短い要約だけでは誤解を招きうるため、実装はこの
/// spec 本文の algorithm に忠実にした
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
pub(crate) fn resolve_inheritance<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    cascaded: &CascadedArena,
    out: &mut Vec<ComputedValues>,
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
        if let Some(candidates) = cascaded.candidates(id) {
            apply_winners(candidates, &mut winners, &mut specified, &custom_properties);
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
        computed.custom_properties = custom_properties.clone();

        // 子へ渡す rem/rlh context。root element の phase 2 + 2.5 が終わった
        // 時点で `root_font_size` / `root_line_height` が確定するので、ここで
        // 初めて `Some` になる。`used_line_height_length` は
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

/// Resolve a deferred declaration for either the element or page cascade.
///
/// `pub(crate)` is limited to the sibling page pipeline; no external API is
/// added. Both callers therefore share the same invalid-at-computed-value-time
/// and shorthand projection behavior.
pub(crate) fn resolve_deferred_value(
    deferred: &DeferredValue,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    let substituted = substitute_vars(&deferred.value, &mut |name| custom_properties.get(name), 0)?;
    let simplified = simplify_math_functions(&substituted)?;
    let mut input = ParserInput::new(simplified.as_ref());
    let mut parser = Parser::new(&mut input);
    let value = parse_value(&deferred.property, &mut parser)?;
    parser.expect_exhausted().ok()?;
    project_deferred_value(value, deferred.key)
}

fn project_deferred_value(
    value: PropertyValue,
    key: crate::property::PropertyKey,
) -> Option<PropertyValue> {
    if value.key() == key {
        return Some(value);
    }
    Some(match value {
        PropertyValue::Padding(sides) => match key {
            crate::property::PropertyKey::PaddingTop => PropertyValue::PaddingTop(sides.top),
            crate::property::PropertyKey::PaddingRight => PropertyValue::PaddingRight(sides.right),
            crate::property::PropertyKey::PaddingBottom => {
                PropertyValue::PaddingBottom(sides.bottom)
            }
            crate::property::PropertyKey::PaddingLeft => PropertyValue::PaddingLeft(sides.left),
            _ => return None,
        },
        PropertyValue::Margin(sides) => match key {
            crate::property::PropertyKey::MarginTop => PropertyValue::MarginTop(sides.top),
            crate::property::PropertyKey::MarginRight => PropertyValue::MarginRight(sides.right),
            crate::property::PropertyKey::MarginBottom => PropertyValue::MarginBottom(sides.bottom),
            crate::property::PropertyKey::MarginLeft => PropertyValue::MarginLeft(sides.left),
            _ => return None,
        },
        // `margin-inline`/`padding-inline`/`margin-block`/`padding-block`
        // logical 2-value shorthand — same projection shape as
        // `Margin`/`Padding` above, fanning out to the 2 physical longhands
        // `crate::rule::expand_deferred`'s key table pairs each shorthand
        // key with (`crate::property::PropertyValue::MarginInline` doc's
        // physical-mapping rationale covers *why* these specific longhands).
        PropertyValue::MarginInline(pair) => match key {
            crate::property::PropertyKey::MarginLeft => PropertyValue::MarginLeft(pair.start),
            crate::property::PropertyKey::MarginRight => PropertyValue::MarginRight(pair.end),
            _ => return None,
        },
        PropertyValue::MarginBlock(pair) => match key {
            crate::property::PropertyKey::MarginTop => PropertyValue::MarginTop(pair.start),
            crate::property::PropertyKey::MarginBottom => PropertyValue::MarginBottom(pair.end),
            _ => return None,
        },
        PropertyValue::PaddingInline(pair) => match key {
            crate::property::PropertyKey::PaddingLeft => PropertyValue::PaddingLeft(pair.start),
            crate::property::PropertyKey::PaddingRight => PropertyValue::PaddingRight(pair.end),
            _ => return None,
        },
        PropertyValue::PaddingBlock(pair) => match key {
            crate::property::PropertyKey::PaddingTop => PropertyValue::PaddingTop(pair.start),
            crate::property::PropertyKey::PaddingBottom => PropertyValue::PaddingBottom(pair.end),
            _ => return None,
        },
        PropertyValue::Outline(outline) => match key {
            crate::property::PropertyKey::OutlineWidth => {
                PropertyValue::OutlineWidth(outline.width)
            }
            crate::property::PropertyKey::OutlineStyle => {
                PropertyValue::OutlineStyle(outline.style)
            }
            crate::property::PropertyKey::OutlineColor => {
                PropertyValue::OutlineColor(outline.color)
            }
            _ => return None,
        },
        PropertyValue::Border(sides) => match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidth(sides.top.width)
            }
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidth(sides.right.width)
            }
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidth(sides.bottom.width)
            }
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidth(sides.left.width)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyle(sides.top.style)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyle(sides.right.style)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyle(sides.bottom.style)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyle(sides.left.style)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColor(sides.top.color)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColor(sides.right.color)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColor(sides.bottom.color)
            }
            crate::property::PropertyKey::BorderLeftColor => {
                PropertyValue::BorderLeftColor(sides.left.color)
            }
            _ => return None,
        },
        PropertyValue::Overflow(pair) => match key {
            crate::property::PropertyKey::OverflowX => PropertyValue::OverflowX(pair.x),
            crate::property::PropertyKey::OverflowY => PropertyValue::OverflowY(pair.y),
            _ => return None,
        },
        PropertyValue::TextDecoration(shorthand) => match key {
            crate::property::PropertyKey::TextDecorationLine => {
                PropertyValue::TextDecorationLine(shorthand.line)
            }
            crate::property::PropertyKey::TextDecorationStyle => {
                PropertyValue::TextDecorationStyle(shorthand.style)
            }
            crate::property::PropertyKey::TextDecorationColor => {
                PropertyValue::TextDecorationColor(shorthand.color)
            }
            _ => return None,
        },
        PropertyValue::Flex(shorthand) => match key {
            crate::property::PropertyKey::FlexGrow => PropertyValue::FlexGrow(shorthand.grow),
            crate::property::PropertyKey::FlexShrink => PropertyValue::FlexShrink(shorthand.shrink),
            crate::property::PropertyKey::FlexBasis => PropertyValue::FlexBasis(shorthand.basis),
            _ => return None,
        },
        PropertyValue::Gap(shorthand) => match key {
            crate::property::PropertyKey::RowGap => PropertyValue::RowGap(shorthand.row),
            crate::property::PropertyKey::ColumnGap => PropertyValue::ColumnGap(shorthand.column),
            _ => return None,
        },
        PropertyValue::PlaceContent(shorthand) => match key {
            crate::property::PropertyKey::AlignContent => {
                PropertyValue::AlignContent(shorthand.align)
            }
            crate::property::PropertyKey::JustifyContent => {
                PropertyValue::JustifyContent(shorthand.justify)
            }
            _ => return None,
        },
        PropertyValue::GridRow(shorthand) => match key {
            crate::property::PropertyKey::GridRowStart => {
                PropertyValue::GridRowStart(shorthand.start)
            }
            crate::property::PropertyKey::GridRowEnd => PropertyValue::GridRowEnd(shorthand.end),
            _ => return None,
        },
        PropertyValue::GridColumn(shorthand) => match key {
            crate::property::PropertyKey::GridColumnStart => {
                PropertyValue::GridColumnStart(shorthand.start)
            }
            crate::property::PropertyKey::GridColumnEnd => {
                PropertyValue::GridColumnEnd(shorthand.end)
            }
            _ => return None,
        },
        PropertyValue::PlaceItems(shorthand) => match key {
            crate::property::PropertyKey::AlignItems => PropertyValue::AlignItems(shorthand.align),
            crate::property::PropertyKey::JustifyItems => {
                PropertyValue::JustifyItems(shorthand.justify)
            }
            _ => return None,
        },
        PropertyValue::PlaceSelf(shorthand) => match key {
            crate::property::PropertyKey::AlignSelf => PropertyValue::AlignSelf(shorthand.align),
            crate::property::PropertyKey::JustifySelf => {
                PropertyValue::JustifySelf(shorthand.justify)
            }
            _ => return None,
        },
        _ => return None,
    })
}

pub(crate) fn simplify_math_functions(input: &str) -> Option<SmolStr> {
    simplify_math_functions_at_depth(input, 0)
}

fn push_bounded(output: &mut String, fragment: &str) -> Option<()> {
    let new_len = output.len().checked_add(fragment.len())?;
    if new_len > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    output.push_str(fragment);
    Some(())
}

const MATH_FUNCTIONS: [&str; 4] = ["calc", "min", "max", "clamp"];

/// Find named CSS functions by walking cssparser's component-value tokens.
/// `Parser::next` skips a function body, so recurse into every block to keep
/// nested `var()`/math functions visible while preserving their byte offsets.
fn find_function_tokens(
    input: &str,
    names: &[&str],
) -> Option<Vec<(SmolStr, usize, usize, usize)>> {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    find_function_tokens_in_parser(&mut parser, names, 0)
}

fn find_function_tokens_in_parser<'i, 't>(
    parser: &mut Parser<'i, 't>,
    names: &[&str],
    depth: usize,
) -> Option<Vec<(SmolStr, usize, usize, usize)>> {
    if depth > MAX_DEFERRED_VALUE_NESTING_DEPTH {
        return None;
    }
    let mut found = Vec::new();
    loop {
        let token_start = parser.position().byte_index();
        let token = match parser.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        match token {
            Token::Function(name) => {
                let open = parser.position().byte_index().checked_sub(1)?;
                let is_target = names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted));
                let decoded_name: SmolStr = name.as_ref().into();
                let mut nested_found = parser
                    .parse_nested_block(|nested| {
                        find_function_tokens_in_parser(nested, names, depth.saturating_add(1))
                            .ok_or_else(|| nested.new_custom_error::<(), ()>(()))
                    })
                    .ok()?;
                if is_target {
                    let close = parser.position().byte_index().checked_sub(1)?;
                    found.push((decoded_name, token_start, open, close));
                }
                found.append(&mut nested_found);
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let mut nested_found = parser
                    .parse_nested_block(|nested| {
                        find_function_tokens_in_parser(nested, names, depth.saturating_add(1))
                            .ok_or_else(|| nested.new_custom_error::<(), ()>(()))
                    })
                    .ok()?;
                found.append(&mut nested_found);
            }
            _ => {}
        }
    }
    Some(found)
}

fn needs_css_token_separator(left: &str, right: &str) -> bool {
    let left = left
        .as_bytes()
        .iter()
        .rev()
        .find(|byte| !byte.is_ascii_whitespace())
        .copied();
    let right = right
        .as_bytes()
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .copied();
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    if left.is_ascii_alphanumeric()
        && (right.is_ascii_alphanumeric() || matches!(right, b'.' | b'%'))
    {
        return true;
    }
    if (left.is_ascii_alphabetic() || matches!(left, b'-' | b'_' | b'\\') || left >= 0x80)
        && (right.is_ascii_alphanumeric() || matches!(right, b'-' | b'_' | b'\\') || right >= 0x80)
    {
        return true;
    }
    if right == b'('
        && (left.is_ascii_alphabetic() || matches!(left, b'-' | b'_' | b'\\') || left >= 0x80)
    {
        return true;
    }
    if left.is_ascii_digit() && (matches!(right, b'-' | b'_' | b'\\') || right >= 0x80) {
        return true;
    }
    if matches!(left, b'+' | b'-') && (right.is_ascii_digit() || right == b'.') {
        return true;
    }
    if left == b'.' && right.is_ascii_digit() {
        return true;
    }
    matches!(left, b'#' | b'@')
        && (right.is_ascii_alphanumeric() || matches!(right, b'-' | b'_' | b'\\') || right >= 0x80)
}

fn simplify_math_functions_at_depth(input: &str, depth: usize) -> Option<SmolStr> {
    if depth > MAX_VARIABLE_RESOLUTION_DEPTH || input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let mut output = String::with_capacity(input.len());
    let mut position = 0;
    let functions = find_function_tokens(input, &MATH_FUNCTIONS)?;
    for (name, name_start, open, close) in functions {
        if name_start < position {
            continue;
        }
        push_bounded(&mut output, &input[position..name_start])?;
        let inner_source = &input[open + 1..close];
        let inner = simplify_math_functions_at_depth(inner_source, depth.saturating_add(1))?;
        let evaluated = evaluate_math_function(name.as_str(), &inner)?;
        push_bounded(&mut output, &evaluated)?;
        if needs_css_token_separator(&output, &input[close + 1..]) {
            push_bounded(&mut output, " ")?;
        }
        position = close.checked_add(1)?;
    }
    push_bounded(&mut output, &input[position..])?;
    Some(output.into())
}

#[derive(Clone, Debug)]
struct MathValue {
    number: f32,
    unit: Option<String>,
}

fn evaluate_math_function(name: &str, input: &str) -> Option<String> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    match name.to_ascii_lowercase().as_str() {
        "calc" => {
            let value = MathParser::new(input).parse()?;
            serialize_math_value(value)
        }
        "min" | "max" => {
            let arguments = split_top_level_commas(input)?;
            if arguments.is_empty() {
                return None; // cov:ignore: split_top_level_commas always returns at least one part when it succeeds.
            }
            let mut values = arguments
                .into_iter()
                .map(|argument| MathParser::new(argument).parse())
                .collect::<Option<Vec<_>>>()?;
            normalize_math_values(&mut values)?;
            let is_min = name.eq_ignore_ascii_case("min");
            let mut selected = values.remove(0);
            for value in values {
                let choose = if is_min {
                    value.number < selected.number
                } else {
                    value.number > selected.number
                };
                if choose {
                    selected = value;
                }
            }
            serialize_math_value(selected)
        }
        "clamp" => {
            let arguments = split_top_level_commas(input)?;
            if arguments.len() != 3 {
                return None;
            }
            let values = arguments
                .into_iter()
                .map(|argument| MathParser::new(argument).parse())
                .collect::<Option<Vec<_>>>()?;
            let mut values = values;
            normalize_math_values(&mut values)?;
            let selected = if values[0].number > values[2].number {
                // CSS Values 4 §10.2: when the bounds are reversed, the
                // minimum wins over the maximum.
                values[0].clone()
            } else {
                MathValue {
                    number: values[1].number.max(values[0].number).min(values[2].number),
                    unit: values[0].unit.clone(),
                }
            };
            serialize_math_value(selected)
        }
        _ => None,
    }
}

fn absolute_length_factor(unit: &str) -> Option<f32> {
    match unit {
        "px" => Some(1.0),
        "in" => Some(96.0),
        "cm" => Some(96.0 / 2.54),
        "mm" => Some(96.0 / 25.4),
        "q" => Some(96.0 / 101.6),
        "pt" => Some(96.0 / 72.0),
        "pc" => Some(16.0),
        _ => None,
    }
}

fn normalize_math_pair(left: &mut MathValue, right: &mut MathValue) -> Option<()> {
    match (&left.unit, &right.unit) {
        (None, None) => Some(()),
        (Some(left_unit), Some(right_unit)) if left_unit == right_unit => Some(()),
        (Some(left_unit), Some(right_unit)) => {
            let left_factor = absolute_length_factor(left_unit)?;
            let right_factor = absolute_length_factor(right_unit)?;
            left.number *= left_factor;
            right.number *= right_factor;
            if !left.number.is_finite() || !right.number.is_finite() {
                return None;
            }
            left.unit = Some("px".to_owned());
            right.unit = Some("px".to_owned());
            Some(())
        }
        _ => None,
    }
}

fn normalize_math_values(values: &mut [MathValue]) -> Option<()> {
    if values.is_empty() {
        return None;
    }
    let first_unit = values[0].unit.clone();
    if values.iter().all(|value| value.unit == first_unit) {
        return Some(());
    }
    if !values.iter().all(|value| {
        value
            .unit
            .as_deref()
            .is_some_and(|unit| absolute_length_factor(unit).is_some())
    }) {
        return None;
    }
    for value in values {
        let factor = absolute_length_factor(value.unit.as_deref()?)?;
        value.number *= factor;
        if !value.number.is_finite() {
            return None;
        }
        value.unit = Some("px".to_owned());
    }
    Some(())
}

fn serialize_math_value(value: MathValue) -> Option<String> {
    if !value.number.is_finite() {
        return None;
    }
    let number = if value.number == 0.0 {
        "0".to_owned()
    } else {
        value.number.to_string()
    };
    let unit = value.unit.as_deref().unwrap_or("");
    if number.len().checked_add(unit.len())? > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    Some(format!("{}{}", number, unit))
}

struct MathParser<'a> {
    input: &'a str,
    position: usize,
    nesting_depth: usize,
}

impl<'a> MathParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            position: 0,
            nesting_depth: 0,
        }
    }

    fn parse(mut self) -> Option<MathValue> {
        let value = self.parse_sum()?;
        self.skip_whitespace()?;
        (self.position == self.input.len()).then_some(value)
    }

    fn parse_sum(&mut self) -> Option<MathValue> {
        let mut value = self.parse_product()?;
        loop {
            self.skip_whitespace()?;
            let Some(&operator) = self.input.as_bytes().get(self.position) else {
                return Some(value);
            };
            if operator != b'+' && operator != b'-' {
                return Some(value);
            }
            let operator_position = self.position;
            if !self.has_css_whitespace_before(operator_position)
                || !self.has_css_whitespace_after(operator_position + 1)
            {
                return None;
            }
            self.position += 1;
            let mut right = self.parse_product()?;
            normalize_math_pair(&mut value, &mut right)?;
            value.number = if operator == b'+' {
                value.number + right.number
            } else {
                value.number - right.number
            };
            if !value.number.is_finite() {
                return None;
            }
        }
    }

    fn parse_product(&mut self) -> Option<MathValue> {
        let mut value = self.parse_primary()?;
        loop {
            self.skip_whitespace()?;
            let operator = match self.input.as_bytes().get(self.position) {
                Some(b'*') | Some(b'/') => self.input.as_bytes()[self.position],
                _ => return Some(value),
            };
            self.position += 1;
            let right = self.parse_primary()?;
            match operator {
                b'*' if value.unit.is_none() => {
                    value = MathValue {
                        number: value.number * right.number,
                        unit: right.unit,
                    }
                }
                b'*' if right.unit.is_none() => value.number *= right.number,
                b'/' if right.unit.is_none() && right.number != 0.0 => value.number /= right.number,
                _ => return None,
            }
            if !value.number.is_finite() {
                return None;
            }
        }
    }

    fn parse_primary(&mut self) -> Option<MathValue> {
        self.skip_whitespace()?;
        if self.input.as_bytes().get(self.position) == Some(&b'(') {
            if self.nesting_depth >= MAX_VARIABLE_RESOLUTION_DEPTH {
                return None;
            }
            self.position += 1;
            self.nesting_depth += 1;
            let value = self.parse_sum();
            self.nesting_depth -= 1;
            let value = value?;
            self.skip_whitespace()?;
            if self.input.as_bytes().get(self.position) != Some(&b')') {
                return None;
            }
            self.position += 1;
            return Some(value);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Option<MathValue> {
        let start = self.position;
        if matches!(
            self.input.as_bytes().get(self.position),
            Some(b'+') | Some(b'-')
        ) {
            self.position += 1;
        }
        let digits_start = self.position;
        while self
            .input
            .as_bytes()
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
        }
        if self.input.as_bytes().get(self.position) == Some(&b'.') {
            self.position += 1;
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
        }
        if self.position == digits_start
            || (self.position == digits_start + 1
                && self.input.as_bytes().get(digits_start) == Some(&b'.'))
        {
            return None;
        }
        if matches!(
            self.input.as_bytes().get(self.position),
            Some(b'e') | Some(b'E')
        ) {
            let exponent_start = self.position;
            self.position += 1;
            if matches!(
                self.input.as_bytes().get(self.position),
                Some(b'+') | Some(b'-')
            ) {
                self.position += 1;
            }
            let exponent_digits = self.position;
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if self.position == exponent_digits {
                self.position = exponent_start;
            }
        }
        let number = self.input[start..self.position].parse::<f32>().ok()?;
        let unit_start = self.position;
        if self.input.as_bytes().get(self.position) == Some(&b'%') {
            self.position += 1;
        } else {
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                self.position += 1;
            }
        }
        let unit = (unit_start != self.position)
            .then(|| self.input[unit_start..self.position].to_ascii_lowercase());
        Some(MathValue { number, unit })
    }

    fn skip_whitespace(&mut self) -> Option<()> {
        loop {
            while self
                .input
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.position += 1;
            }
            if self.input.as_bytes().get(self.position..self.position + 2) == Some(b"/*") {
                self.position = skip_css_comment(self.input, self.position)?;
                continue;
            }
            return Some(());
        }
    }

    fn has_css_whitespace_before(&self, position: usize) -> bool {
        if position == 0 {
            return false;
        }
        if self.input.as_bytes()[position - 1].is_ascii_whitespace() {
            return true;
        }
        self.input[..position].ends_with("*/")
    }

    fn has_css_whitespace_after(&self, position: usize) -> bool {
        self.input
            .as_bytes()
            .get(position)
            .is_some_and(u8::is_ascii_whitespace)
            || self.input.as_bytes().get(position..position + 2) == Some(b"/*")
    }
}

fn split_top_level_commas(input: &str) -> Option<Vec<&str>> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut closers = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        match bytes[position] {
            b'\'' | b'"' => position = skip_css_string(input, position)?,
            b'/' if bytes.get(position + 1) == Some(&b'*') => {
                position = skip_css_comment(input, position)?
            }
            b'\\' => position = skip_css_escape(input, position)?,
            b'(' | b'[' | b'{' => {
                if closers.len() >= MAX_DEFERRED_VALUE_NESTING_DEPTH {
                    return None;
                }
                let closer = if bytes[position] == b'(' {
                    b')'
                } else if bytes[position] == b'[' {
                    b']'
                } else {
                    b'}'
                };
                closers.push(closer);
                position += 1;
            }
            b')' | b']' | b'}' if closers.last() == Some(&bytes[position]) => {
                closers.pop();
                position += 1;
            }
            b')' | b']' | b'}' if closers.is_empty() => return None,
            b',' if closers.is_empty() => {
                parts.push(input[start..position].trim());
                start = position + 1;
                position += 1;
            }
            _ => position += 1,
        }
    }
    if !closers.is_empty() {
        return None;
    }
    parts.push(input[start..].trim());
    parts.iter().all(|part| !part.is_empty()).then_some(parts)
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
fn apply_winners(
    candidates: &[CascadedDecl],
    winners: &mut Vec<Option<RankedDecl>>,
    specified: &mut SpecifiedValues,
    custom_properties: &CustomPropertyEnvironment,
) {
    pick_winners(candidates, winners);
    for slot in winners.iter_mut() {
        if let Some(winner) = slot.take() {
            let value = &candidates[winner.idx].0;
            let value = match value {
                PropertyValue::Deferred(deferred) => {
                    resolve_deferred_value(deferred, custom_properties)
                }
                _ => Some(value.clone()),
            };
            if let Some(value) = value {
                apply_value(value, specified);
            }
        }
    }
}

// CSS Variables 1 §3.3 requires a UA-defined expansion limit to prevent
// exponential substitution blowups. The depth bound is an additional stack
// guard for the mutually recursive resolver/substituter paths.
const MAX_VARIABLE_RESOLUTION_DEPTH: usize = MAX_DEFERRED_VALUE_NESTING_DEPTH;

/// Maximum number of uncached custom-property expansions in one map resolve.
/// Memoization handles shared DAG branches; this second bound caps the work
/// spent on a large, mostly-unique dependency graph.
const MAX_VARIABLE_RESOLUTION_STEPS: usize = 16 * 1024;

struct VariableResolutionBudget {
    remaining: usize,
    exhausted: bool,
}

impl VariableResolutionBudget {
    fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            exhausted: false,
        }
    }

    fn consume(&mut self) -> bool {
        if self.remaining == 0 {
            self.exhausted = true;
            return false;
        }
        self.remaining -= 1;
        true
    }
}

fn resolve_custom_properties(
    inherited: &Arc<CustomPropertyEnvironment>,
    candidates: &[CustomCascadedDecl],
) -> Arc<CustomPropertyEnvironment> {
    let mut winners: HashMap<SmolStr, (CustomProperty, RankedDecl)> = HashMap::new();
    for (idx, (value, important, origin, specificity, source_order)) in
        candidates.iter().enumerate()
    {
        let candidate = RankedDecl {
            rank: cascade_rank(*origin, *important),
            specificity: *specificity,
            source_order: *source_order,
            idx,
        };
        let replace = winners
            .get(&value.name)
            .is_none_or(|(_, existing)| beats(candidate, *existing));
        if replace {
            winners.insert(value.name.clone(), (value.clone(), candidate));
        }
    }

    let local: HashMap<SmolStr, SmolStr> = winners
        .into_iter()
        .map(|(name, (value, _))| (name, value.value))
        .collect();
    resolve_custom_property_environment(inherited, &local)
}

/// Resolve one element or page context's local custom properties against a
/// shared inherited environment.
///
/// The returned environment owns only local entries. A `None` entry records a
/// declared-but-invalid custom property, which is a guaranteed-invalid value
/// and therefore shadows an inherited value while allowing `var()` fallback.
pub(crate) fn resolve_custom_property_environment(
    inherited: &Arc<CustomPropertyEnvironment>,
    local: &HashMap<SmolStr, SmolStr>,
) -> Arc<CustomPropertyEnvironment> {
    if local.is_empty() {
        return inherited.clone();
    }
    let entries = resolve_local_custom_properties(inherited.as_ref(), local);
    CustomPropertyEnvironment::from_local(inherited, entries)
}

fn resolve_local_custom_properties(
    inherited: &CustomPropertyEnvironment,
    local: &HashMap<SmolStr, SmolStr>,
) -> HashMap<SmolStr, Option<SmolStr>> {
    let names: Vec<SmolStr> = local.keys().cloned().collect();
    let mut resolver = CustomPropertyResolver {
        local,
        inherited,
        cycle_members: find_cycle_members(local),
        memo: HashMap::new(),
        resolving: Vec::new(),
        budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
    };
    let mut entries = HashMap::with_capacity(local.len());
    for name in names {
        entries.insert(name.clone(), resolver.resolve(&name, 0));
    }
    entries
}

struct CustomPropertyResolver<'a> {
    local: &'a HashMap<SmolStr, SmolStr>,
    inherited: &'a CustomPropertyEnvironment,
    cycle_members: HashSet<SmolStr>,
    memo: HashMap<(SmolStr, usize), Option<SmolStr>>,
    resolving: Vec<SmolStr>,
    budget: VariableResolutionBudget,
}

impl CustomPropertyResolver<'_> {
    fn resolve(&mut self, name: &str, depth: usize) -> Option<SmolStr> {
        if depth > MAX_VARIABLE_RESOLUTION_DEPTH {
            return None;
        }
        let name: SmolStr = name.into();
        let key = (name.clone(), depth);
        if let Some(value) = self.memo.get(&key) {
            return value.clone();
        }
        if !self.budget.consume() {
            return None;
        }
        if self.cycle_members.contains(&name) {
            return None;
        }
        let Some(raw) = self.local.get(&name).cloned() else {
            return self.inherited.get(&name);
        };
        if self.resolving.iter().any(|current| current == &name) {
            return None;
        }

        self.resolving.push(name.clone());
        let resolved = substitute_vars(
            &raw,
            &mut |reference| self.resolve(reference, depth.saturating_add(1)),
            depth,
        );
        self.resolving.pop();
        if !self.budget.exhausted {
            self.memo.insert(key, resolved.clone());
        }
        if self.budget.exhausted {
            None
        } else {
            resolved
        }
    }
}

fn find_cycle_members(local: &HashMap<SmolStr, SmolStr>) -> HashSet<SmolStr> {
    let mut dependencies: HashMap<SmolStr, Vec<SmolStr>> = HashMap::new();
    for (name, value) in local {
        let mut references = Vec::new();
        if collect_var_references(value, &mut references).is_ok() {
            references.retain(|reference| local.contains_key(reference));
            dependencies.insert(name.clone(), references);
        } else {
            dependencies.insert(name.clone(), Vec::new());
        }
    }

    // Iterative depth-first search keeps a stylesheet with a long variable
    // chain from consuming the Rust call stack while still identifying every
    // back-edge cycle in the per-element dependency graph.
    let mut states: HashMap<SmolStr, u8> = HashMap::new();
    let mut cycle_members = HashSet::new();
    for start in local.keys() {
        if states.get(start).copied().unwrap_or_default() != 0 {
            continue;
        }
        let mut stack = vec![(start.clone(), 0usize)];
        let mut path = vec![start.clone()];
        states.insert(start.clone(), 1);
        while let Some((node, next_index)) = stack.last_mut() {
            let node_name = node.clone();
            let deps = dependencies
                .get(&node_name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if *next_index >= deps.len() {
                states.insert(node_name, 2);
                stack.pop();
                path.pop();
                continue;
            }
            let target = deps[*next_index].clone();
            *next_index += 1;
            match states.get(&target).copied().unwrap_or_default() {
                0 => {
                    states.insert(target.clone(), 1);
                    path.push(target.clone());
                    stack.push((target, 0));
                }
                1 => {
                    if let Some(cycle_start) = path.iter().position(|name| name == &target) {
                        cycle_members.extend(path[cycle_start..].iter().cloned());
                    }
                }
                _ => {}
            }
        }
    }
    cycle_members
}

fn collect_var_references(input: &str, references: &mut Vec<SmolStr>) -> Result<(), ()> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES || !css_literals_are_well_formed(input) {
        return Err(());
    }
    let functions = find_function_tokens(input, &["var"]).ok_or(())?;
    for (_, _, open, close) in functions {
        let (name, _) = split_var_arguments(&input[open + 1..close]).ok_or(())?;
        references.push(name);
    }
    Ok(())
}

fn substitute_vars(
    input: &str,
    resolve: &mut impl FnMut(&str) -> Option<SmolStr>,
    depth: usize,
) -> Option<SmolStr> {
    let mut budget = VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS);
    substitute_vars_with_budget(input, resolve, depth, &mut budget)
}

fn substitute_vars_with_budget(
    input: &str,
    resolve: &mut impl FnMut(&str) -> Option<SmolStr>,
    depth: usize,
    budget: &mut VariableResolutionBudget,
) -> Option<SmolStr> {
    if depth > MAX_VARIABLE_RESOLUTION_DEPTH
        || input.len() > MAX_SUBSTITUTED_VALUE_BYTES
        || !css_literals_are_well_formed(input)
    {
        return None;
    }
    let mut output = String::with_capacity(input.len());
    let mut position = 0;
    let functions = find_function_tokens(input, &["var"])?;
    for (_, name_start, open, close) in functions {
        if name_start < position {
            continue;
        }
        if !budget.consume() {
            return None;
        }
        push_bounded(&mut output, &input[position..name_start])?;
        let inner = &input[open + 1..close];
        let (name, fallback) = split_var_arguments(inner)?;
        let replacement = match resolve(name.as_str()) {
            Some(value) => value,
            None => match fallback {
                Some(fallback) => {
                    substitute_vars_with_budget(fallback, resolve, depth.saturating_add(1), budget)?
                }
                None => return None,
            },
        };
        push_bounded(&mut output, &replacement)?;
        if needs_css_token_separator(&output, &input[close + 1..]) {
            // Substitution preserves the original component-value token
            // boundaries. Add a separator only where reparsing the bounded
            // serialization would otherwise merge adjacent tokens.
            push_bounded(&mut output, " ")?;
        }
        position = close.checked_add(1)?;
    }
    push_bounded(&mut output, &input[position..])?;
    Some(output.into())
}

fn css_literals_are_well_formed(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut position = 0;
    while position < bytes.len() {
        if bytes[position] == b'\'' || bytes[position] == b'"' {
            let Some(end) = skip_css_string(input, position) else {
                return false;
            };
            position = end;
            continue;
        }
        if bytes[position] == b'/' && bytes.get(position + 1) == Some(&b'*') {
            let Some(end) = skip_css_comment(input, position) else {
                return false;
            };
            position = end;
            continue;
        }
        let Some(character) = input[position..].chars().next() else {
            return false; // cov:ignore: a valid Rust str cannot end between UTF-8 scalar values.
        };
        position += character.len_utf8();
    }
    true
}

fn skip_css_string(input: &str, start: usize) -> Option<usize> {
    let quote = input.as_bytes()[start];
    let bytes = input.as_bytes();
    let mut position = start + 1;
    while position < bytes.len() {
        match bytes[position] {
            b'\\' => position = skip_css_escape(input, position)?,
            byte if byte == quote => return Some(position + 1),
            _ => position += 1,
        }
    }
    None
}

fn skip_css_comment(input: &str, start: usize) -> Option<usize> {
    input[start + 2..]
        .find("*/")
        .map(|offset| start + 2 + offset + 2)
}

fn split_var_arguments(input: &str) -> Option<(SmolStr, Option<&str>)> {
    if input.len() > MAX_SUBSTITUTED_VALUE_BYTES {
        return None;
    }
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut position = 0;
    while position < bytes.len() {
        match bytes[position] {
            b'\'' | b'"' => position = skip_css_string(input, position)?,
            b'/' if bytes.get(position + 1) == Some(&b'*') => {
                position = skip_css_comment(input, position)?
            }
            b'(' => {
                depth += 1;
                position += 1;
            }
            b')' => {
                depth = depth.checked_sub(1)?;
                position += 1;
            }
            b'\\' => position = skip_css_escape(input, position)?,
            b',' if depth == 0 => {
                let name = parse_custom_property_reference(&input[..position])?;
                return Some((name, Some(input[position + 1..].trim())));
            }
            _ => position += 1,
        }
    }
    let name = parse_custom_property_reference(input)?;
    Some((name, None))
}

fn parse_custom_property_reference(input: &str) -> Option<SmolStr> {
    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    let name = parser.expect_ident_cloned().ok()?;
    if !is_custom_property_name(name.as_ref()) {
        return None;
    }
    parser.expect_exhausted().ok()?;
    Some(name.as_ref().into())
}

fn is_css_whitespace_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0C' | b'\r')
}

fn is_css_newline_byte(byte: u8) -> bool {
    matches!(byte, b'\n' | b'\x0C' | b'\r')
}

fn skip_css_escape(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.get(start) != Some(&b'\\') {
        return None;
    }
    let mut position = start + 1;
    let first = *bytes.get(position)?;
    if is_css_newline_byte(first) {
        return None;
    }
    if first.is_ascii_hexdigit() {
        let mut digits = 0;
        while digits < 6 && bytes.get(position).is_some_and(u8::is_ascii_hexdigit) {
            digits += 1;
            position += 1;
        }
        if bytes
            .get(position)
            .is_some_and(|byte| is_css_whitespace_byte(*byte))
        {
            if bytes.get(position) == Some(&b'\r') && bytes.get(position + 1) == Some(&b'\n') {
                position += 2;
            } else {
                position += 1;
            }
        } // cov:ignore: this block terminator has no executable statement; both escape branches are covered above.
        return Some(position);
    }
    let character = input[position..].chars().next()?;
    Some(position + character.len_utf8())
}

/// property key ごとに勝者 declaration を pick (specificity + !important + source order)。
///
/// 結果は返さず `best` に書く。`best` は [`PropertyKey`] の discriminant を
/// そのまま index にした **direct-address table** で、`best[k as usize]` が
/// key `k` の勝者 (= `candidates` 内 index) を持つ。
///
/// # なぜ `HashMap` を返さないのか
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
    // - rank 高い方が勝つ (順序と正確な値は `cascade_rank` doc 参照 — 当初の
    //   UA/Author の 2 段から現在の UA/User/AuthorPresentationalHint/Author
    //   の 4 段まで拡張済み)
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
/// 行を誤って踏んでいた (詳細: `crate::property::parse_font_weight` doc)。
///
/// # 非有限 `inherited` (`NaN` / `±Inf`) — 本関数は guard しない
///
/// `u16` だった頃は非有限が型で構造的に排除されていたが、`f32` 化で
/// finiteness は「型で保証」から「呼び出し元の値
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
/// **本関数自体には runtime guard を追加しない** (「非有限 / 範囲外 f32 の
/// guard は sink 境界に置く、resolve 層には置かない」という既存方針を
/// 踏襲)。上記の非対称処理は
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
/// pin する。
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
        | PropertyValue::CounterReset(_)
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
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
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
        | PropertyValue::TextDecoration(_)
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
        | PropertyValue::BoxShadow(_)
        | PropertyValue::Outline(_)
        | PropertyValue::OutlineWidth(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
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
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_)) => v,
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
        // (`winner_does_not_leak_into_next_sibling` test がこの "1 回だけ" を pin
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
        // counter-* は将来の GCPM (paged media generated content) 対応に
        // 向けた足場 — parse 結果をそのまま computed value に格納。counter
        // tree の実際の resolve は将来の本実装で行う。
        PropertyValue::CounterReset(v) => target.counter_reset = v,
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
            PositionValue::Static => {}
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
        // CSS Text 3 §8.1 text-indent — **inherited**. `Length` is `Copy`,
        // by-value assignment suffices (sibling `TextAlign`/`Direction`
        // pattern). Absolutization (`em`/`rem`/`%` etc.) happens later in
        // `SpecifiedValues::finalize` / `finalize_as_root`, mirroring
        // `padding`'s phase-3 handling — the only difference from `padding`
        // is that this field is inherited, so `SpecifiedValues::inherit_from`
        // (not this function) is what seeds a child with no winner of its
        // own.
        PropertyValue::TextIndent(v) => target.text_indent = v,
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
        // 直接叩いて pin (`Margin` arm 上の
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
        PropertyValue::Width(v) => target.width = v,
        // CSS Sizing 3 §3.1.1 height。sibling `Width` /
        // `Padding*` / `Margin*` と同じ per-node winner 直接代入 (non-inherited、
        // `LengthOrAuto` は Copy)。resolve (`Percent` / `Auto` の実 layout 高さ
        // 計算) は下流責務。
        PropertyValue::Height(v) => target.height = v,
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
        PropertyValue::LetterSpacing(ls) => target.letter_spacing = ls,
        // CSS Text 3 §7.1。直上の LetterSpacing arm と同型。
        PropertyValue::WordSpacing(ws) => target.word_spacing = ws,
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
        PropertyValue::Quotes(v) => target.quotes = v,
        // CSS Text Decoration Module Level 3 §4。specified 表現
        // (`Arc<Vec<TextShadowItem>>`) のまま格納 — 絶対化 (各 item の length
        // 3 本) は phase 3 (`SpecifiedValues::finalize` / `absolutize_with`)
        // に委ねる (`LetterSpacing`/`FlexBasis` arm と同じ handling)。
        // inherited property のため、cascade winner が無い child は
        // `inherit_from` で親値 (lift 済み) を引き継ぐ。`Arc` は Clone が
        // bump のみなので by-value 代入で十分。
        PropertyValue::TextShadow(shadows) => target.text_shadow = shadows,
        PropertyValue::BorderRadius(v) => target.border_radius = v,
        PropertyValue::BoxShadow(shadows) => target.box_shadow = shadows,
        PropertyValue::Outline(v) => target.outline = v,
        PropertyValue::OutlineWidth(v) => target.outline.width = v,
        PropertyValue::OutlineStyle(v) => target.outline.style = v,
        PropertyValue::OutlineColor(v) => target.outline.color = v,
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
        // These values are resolved before ordinary winners reach this
        // function. Keeping an explicit no-op makes direct internal callers
        // panic-free without allowing raw deferred data into a computed field.
        PropertyValue::CustomProperty(_) | PropertyValue::Deferred(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::INITIAL_FONT_SIZE_PX;
    use crate::property::CssColor;
    use crate::property::DisplayValue;
    use crate::property::{
        Border, BorderColor, BorderStyle, Length, LengthOrAuto, OutlineColor, OutlineStyle,
        OverflowValue, OverflowXY, PropertyKey, Sides, TextDecorationColor, TextDecorationLine,
        TextDecorationShorthand, TextDecorationStyle, TextShadowColor,
    };
    use crate::resolve::{
        ComputedBorder, ComputedBorderRadius, ComputedBoxShadowItem, ComputedLength,
        ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLineHeight,
        ComputedTabSize, ComputedTextShadow,
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

    // ── class / id / attribute selector matching ──
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
        // Acceptance: `.chapter-title { font-weight:
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

    /// Acceptance: quirks-mode 3 状態 × class-selector
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
        // Acceptance: `#header { ... }` applied to
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

    /// Acceptance: quirks-mode 3 状態 × id-selector case
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
        // Acceptance: `[data-foo]` matches any
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
        // Pins `TestDoc`'s own `StyleElement::attr` override
        // (`test_dom.rs`), which deliberately keeps the older, stricter
        // "empty value is normalised to `None`" behavior as a
        // simplification local to this mock. Against `TestDoc`,
        // `data-foo=""` reads back as attribute-absent, so `[data-foo]`
        // does not match here.
        //
        // **This is `TestDoc`-only, not the real DOM's behavior.**
        // `raikiri-dom::dom_impl::ElementRef::attr` (the real DOM impl)
        // tracks attribute presence independent of value, so
        // `data-foo=""` does match `[data-foo]` there — this test's name
        // and outcome describe the mock's narrower contract, not a general
        // engine-level accepted-baseline divergence from CSS Selectors L4.
        // See `StyleElement::attr`'s trait doc (style_dom.rs) for the full
        // contract and this divergence's rationale.
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
        // `compound_matches` arm (`Component::AttributeInNoNamespace`, not
        // `..Exists`): `TestDoc`'s own `StyleElement::attr` override
        // (`test_dom.rs`) collapses `foo=""` into `None` before the
        // with-value arm's `match elem.attr(...) { Some(..) => ..,
        // None => false }` ever runs, so it takes the `None => false`
        // branch regardless of the selector's own value operand.
        //
        // **This is `TestDoc`-only, not the real DOM's behavior.** Per CSS
        // Selectors L4 (<https://www.w3.org/TR/selectors-4/#attribute-selectors>),
        // `[data-foo=""]` should match an element whose `data-foo` value is
        // exactly the empty string, and `raikiri-dom::dom_impl::ElementRef::attr`
        // (the real DOM impl) does support that — it tracks presence
        // independent of value. Only `TestDoc`'s deliberately-simplified
        // mock still collapses `foo=""` to absent; this test's name and
        // outcome describe that mock, not a general engine-level
        // accepted-baseline divergence. See `StyleElement::attr`'s trait
        // doc (style_dom.rs) for the full contract and this divergence's
        // rationale.
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
        // Acceptance: `[data-foo="bar"]` exact-match
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
        // `inline_style_source()`; `TestElementRef::attr`
        // does this, so `[style]` — an ordinary
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

    /// CSS Cascading and Inheritance Level 4 §6.1 "Cascade Sorting Order"
    /// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>, Specificity step
    /// verbatim: "declarations that do not belong to a style rule (such as
    /// the contents of a style attribute) are considered to have a
    /// specificity higher than any selector." Unlike
    /// [`inline_specificity_exceeds_max_reachable_packed_specificity`],
    /// which pins the `INLINE_SPECIFICITY` constant against a
    /// packed-specificity numeric ceiling, this drives the full
    /// [`cascade()`] pipeline end to end: a real `build_rule_tree` +
    /// `cascade` run against a deliberately high-specificity author
    /// selector (id + 3 classes) matched against an inline `style`
    /// attribute on the same element.
    #[test]
    fn inline_style_beats_maximally_specific_selector_via_cascade() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#a.b.c.d { color: red }");
        let p = doc.push_element(0, "p", Some("color: blue"));
        doc.set_attr(p, "id", "a");
        doc.set_attr(p, "class", "b c d");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].color, BLUE,
            "inline style must win over #a.b.c.d despite its high selector specificity"
        );
    }

    // --- descendant / child combinator ---
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
        // Acceptance: `.chapter h2` applied to
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
        // Acceptance: `ol > li` applies to a
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
        // `div + p` requires `div`/`p` to be
        // *siblings* (CSS Selectors L4 adjacent-sibling-combinators,
        // "share the same parent"). Here `p` is instead a *child* of
        // `div` — the ancestor relationship must NOT satisfy the sibling
        // combinator, even though `div` is literally `ancestors.last()`.
        // Directly exercises `match_combinator_chain`'s `NextSibling` arm
        // (this test predates sibling-combinator support, when it was named
        // `match_combinator_chain_rejects_unsupported_combinator_via_safety_net`,
        // and `+` fell through the `_ => false` safety net for a different
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

    // ── Sibling combinators ──

    #[test]
    fn adjacent_sibling_combinator_applies_only_to_immediately_following_sibling() {
        // Acceptance: `h2 + p` applies to the `<p>`
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
        // Acceptance: `h2 ~ p` applies to every
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
        // Acceptance, literal form: both `h2 + p`
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
        // resumable* `D::ChildIter` - not covered by any existing test:
        // doubling `~`
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
        // `is_supported_selector_list`.
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

    /// Builds a `depth`-deep uniformly-tagged `<div>` chain (root's first
    /// child down to the deepest, returned as `(doc, deepest_id,
    /// ancestors_of_deepest)`) and a `div`-only descendant-combinator
    /// selector with `compounds` compounds — the shape
    /// [`match_combinator_chain`]'s "Memoization" doc empirically measures.
    /// `compounds == depth` is the "exact fit" case (every compound has an
    /// ancestor slot; matches almost immediately via the greedy
    /// nearest-candidate path). `compounds == depth + 1` is the minimal
    /// *unsatisfiable* case (one ancestor short of what the selector
    /// needs) — this is the one that drove `2^(depth-1)`
    /// [`match_from_element`] calls before this test's fix (see that doc
    /// for the derivation and measured pre-fix timings at smaller depths).
    fn uniform_div_chain_and_selector(
        depth: usize,
        compounds: usize,
    ) -> (
        TestDoc,
        StyleNodeId,
        Vec<StyleNodeId>,
        SelectorList<RaikiriSelectorImpl>,
    ) {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        let mut ids = Vec::with_capacity(depth);
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
            ids.push(parent);
        }
        let target_id = StyleNodeId::new(*ids.last().expect("depth > 0") as u64);
        let ancestors: Vec<StyleNodeId> = ids[..ids.len() - 1]
            .iter()
            .map(|&id| StyleNodeId::new(id as u64))
            .collect();
        let selector = vec!["div"; compounds].join(" ");
        let list = crate::parse_selector_list(&selector).expect("selector parses");
        (doc, target_id, ancestors, list)
    }

    /// This is the DoS this branch fixes: an attacker-controlled markup
    /// depth matched against an attacker-controlled (or even entirely
    /// ordinary, e.g. deeply nested `<div>`s) stylesheet could drive
    /// [`Combinator::Descendant`] backtracking to `2^(depth-1)`
    /// [`match_from_element`] calls — CPU exhaustion with no crash, no
    /// stack limit involved (distinct from `deep_child_combinator_chain_
    /// small_stack_no_overflow`'s native-stack-overflow concern above,
    /// which the explicit-`Vec`-stack rewrite already fixed independently
    /// of this test). Depth 200 with the pre-fix (unmemoized) backtracking
    /// search would need on the order of `2^198` `match_from_element`
    /// calls to conclude "no match" — not slow, simply never finishing on
    /// any real machine (`match_combinator_chain`'s "Memoization" doc has
    /// the exact recurrence, confirmed empirically at smaller depths to
    /// match `2^(k-1)` scaling).
    ///
    /// Post-fix (memoized), the same 200-deep unsatisfiable case is bounded
    /// polynomially in the ancestor/compound counts and completes in a
    /// small fraction of a second — asserted here with a generous 5s
    /// bound (not a tight one) so this doesn't flake under a loaded gate
    /// run; the point is "no longer exponential", not a precise benchmark
    /// (formal benchmarking is out of scope for this change).
    #[test]
    fn descendant_combinator_deep_unsatisfiable_chain_does_not_explode() {
        let depth = 200;
        let (doc, target_id, ancestors, list) = uniform_div_chain_and_selector(depth, depth + 1);
        let node = doc.node(target_id).unwrap();
        let elem = node.as_element().unwrap();

        let start = std::time::Instant::now();
        let result = match_complex_selector_list(
            &list,
            &doc,
            &elem,
            target_id,
            &ancestors,
            StyleQuirksMode::NoQuirks,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            result,
            None,
            "a {}-compound div-only selector against a {depth}-deep div chain is \
             genuinely unsatisfiable (one compound more than there are ancestor \
             slots) — must resolve to no match, not merely resolve fast",
            depth + 1
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "match_combinator_chain's Descendant backtracking must be memoized \
             to polynomial time — took {elapsed:?} for a {depth}-deep unsatisfiable \
             chain, which the pre-fix exponential backtracking could never \
             realistically finish at all"
        );
    }

    /// Sibling case of the test directly above — same pathological shape
    /// ([`match_combinator_chain`]'s "Memoization" doc notes the memo
    /// bounds `Combinator::LaterSibling` backtracking identically, as a
    /// consequence of being keyed generically rather than per-combinator).
    /// `* ~ * ~ ... ~ *` (general-sibling combinators) against a run of
    /// uniformly-matching siblings, with one compound more than there are
    /// preceding siblings to satisfy them — the minimal unsatisfiable case
    /// on the sibling axis instead of the ancestor axis.
    #[test]
    fn later_sibling_combinator_deep_unsatisfiable_run_does_not_explode() {
        let sibling_count = 200;
        let mut doc = TestDoc::new();
        for _ in 0..sibling_count {
            doc.push_element(0, "div", None);
        }
        let target = doc.push_element(0, "div", None);
        let target_id = StyleNodeId::new(target as u64);
        // `target` has `sibling_count` preceding `<div>` siblings and no
        // ancestor element (root-level, same layout the existing sibling
        // acceptance tests above use to exercise the `ancestors.last() ==
        // None -> dom.root_id()` fallback).
        let selector = vec!["div"; sibling_count + 2].join(" ~ ");
        let list = crate::parse_selector_list(&selector).expect("selector parses");
        let node = doc.node(target_id).unwrap();
        let elem = node.as_element().unwrap();

        let start = std::time::Instant::now();
        let result = match_complex_selector_list(
            &list,
            &doc,
            &elem,
            target_id,
            &[],
            StyleQuirksMode::NoQuirks,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            result,
            None,
            "a {}-compound div-only general-sibling selector against {sibling_count} \
             preceding siblings is genuinely unsatisfiable (one compound more than \
             there are preceding-sibling slots) — must resolve to no match",
            sibling_count + 2
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "LaterSibling backtracking must be bounded by the same memo as \
             Descendant — took {elapsed:?} for {sibling_count} unsatisfiable \
             siblings"
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
        // (this test was renamed alongside the function when `dom`/
        // `ancestors` args were added; the `elem_id` arg was added later,
        // matching the current signature).
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

    // ── `:lang()` / `:dir()` ──

    /// Acceptance: `:lang(ja) { font-family: serif-ja }` applies to an
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

    /// Acceptance (the actual "top gap" this closes): `:lang(ja)`
    /// must apply to an element with **no lang attribute of its own**, whose
    /// language is inherited from an ancestor (`<html lang="ja">` in the
    /// example below) — the ancestor-walk this reuses from the
    /// descendant/child combinator matching.
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
    /// any ancestor carries a `lang` attribute at all, so the content
    /// language resolves to the empty string (`effective_language`'s
    /// exhausted-ancestor-chain terminal case) — a non-empty range like
    /// `ja` must not match that (`subtags_match`'s first-subtag comparison
    /// step, CSS Selectors L4 §7.2 — see `effective_language`'s doc). This
    /// is `:lang(ja)`-range-specific: contrast with
    /// `lang_empty_string_range_matches_when_no_lang_anywhere_in_ancestor_chain`,
    /// where the same no-lang-anywhere element correctly *does* match the
    /// literal empty-string range `:lang("")`.
    #[test]
    fn lang_ja_range_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
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

    /// HTML LS §3.2.6.2's own-`lang`
    /// step is scoped to "an HTML element or an element in the SVG
    /// namespace" (quoted in full on `effective_language`'s doc) — a MathML
    /// element's own `lang` attribute must NOT be consulted, unlike SVG's
    /// (`own_html_or_svg_lang_attribute`'s 2-element allowlist). Concrete
    /// spec example: `<html lang="en"><math lang="ja">…`
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

    /// `:lang()`/`:dir()` on a non-rightmost compound (low severity, optional) —
    /// the left side of a
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
    /// and — the actual differentiator from `[dir=rtl]` — a descendant with
    /// no `dir` of its own inherits that directionality
    /// (`resolve_directionality`'s ancestor walk).
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

    /// Mirror of the `rtl`-ancestor case above with an `ltr` ancestor
    /// instead — [`own_explicit_direction`]'s `Ltr` arm is only reachable
    /// via [`resolve_directionality`]'s ancestor-walk loop (an element's
    /// *own* `ltr`/`rtl` state is read via [`own_dir_attribute_state`]
    /// directly), so this is the only test that exercises it; the `rtl`
    /// sibling test above only exercises the loop's `Rtl` arm. An `rtl`
    /// *grandparent* wraps the `ltr` parent specifically so this test can
    /// fail: without it, a broken `Ltr` arm would still leave `span`
    /// resolving to `'ltr'` via the loop-exhausted default, indistinguishable
    /// from the arm working — with the `rtl` grandparent present, a broken
    /// `Ltr` arm instead lets the walk continue past the `ltr` parent to the
    /// `rtl` grandparent, flipping `span` to `rtl`.
    #[test]
    fn dir_matches_explicit_ltr_and_inherits_to_descendant_without_own_dir() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let section = doc.push_element(0, "section", None);
        doc.set_attr(section, "dir", "rtl"); // grandparent, see doc above
        let article = doc.push_element(section, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None); // no dir of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            ":dir(ltr) must match a descendant with no dir attribute of its \
             own, inheriting from its dir=\"ltr\" ancestor rather than \
             continuing past it to the dir=\"rtl\" grandparent"
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

    /// Selectors L4 §7.2's own example, quoted on [`language_range_matches`]'s
    /// doc: "`:lang(åå)` would not match, because it contain[s] non-ASCII
    /// characters so is ill-formed." Checked against several `lang` values,
    /// including `åå` itself, to pin "never matches any element, regardless
    /// of its lang attribute value" (not merely "doesn't happen to match
    /// this particular content language").
    #[test]
    fn language_range_matches_rejects_non_ascii_ill_formed_range() {
        for content_language in ["en", "en-US", "åå", ""] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                !language_range_matches("åå", content_language),
                "ill-formed range :lang(åå) must never match content \
                 language {content_language:?}"
            );
        }
    }

    /// The other half of Selectors L4 §7.2's paired example (same fetch as
    /// above): "`:lang(qq)` could match, even though qq is not a
    /// registered language code." `qq` is well-formed (2 ASCII alpha
    /// characters) but not IANA-registered — pins that this
    /// implementation's bar is well-formedness (RFC 5646 §2.1's ABNF), not
    /// full BCP47 validity (well-formed *and* every subtag registered),
    /// matching the spec's own worked example rather than a stricter
    /// registry check a future change might otherwise "helpfully" add.
    #[test]
    fn language_range_matches_well_formed_unregistered_subtag_can_match() {
        assert!(language_range_matches("qq", "qq"));
        assert!(language_range_matches("qq", "qq-Latn"));
    }

    /// Well-formedness rejects an overlong subtag (RFC 5646 §2.1: "All
    /// subtags have a maximum length of eight characters") on either side —
    /// covers [`is_well_formed_language_tag`]'s later-subtag branch, which
    /// the non-ASCII range test above does not reach (that one fails on the
    /// first-subtag check instead).
    #[test]
    fn language_range_matches_rejects_overlong_subtag_on_either_side() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !language_range_matches("en", "en-abcdefghi"),
            "9-character subtag exceeds RFC 5646's 8-character maximum"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !language_range_matches("en-abcdefghi", "en-US"),
            "same overlong-subtag rejection, this time on the range side"
        );
    }

    /// A double hyphen splits into an empty subtag, which fits no BCP47
    /// subtag production regardless of position.
    #[test]
    fn language_range_matches_rejects_empty_subtag_from_double_hyphen() {
        assert!(!language_range_matches("en", "en--US"));
    }

    /// RFC 5646 §4.5 step 3 canonicalization (deprecated primary language
    /// subtag -> registry `Preferred-Value`) is a genuine false-negative
    /// source without it: `iw` is the deprecated form of `he` (IANA
    /// Language Subtag Registry, `Type: language`, `Subtag: iw`,
    /// `Deprecated`, `Preferred-Value: he`), so a conformant UA must match
    /// `:lang(he)` against `lang="iw"` **and** `:lang(iw)` against
    /// `lang="he"` — canonicalization runs on both the range and the
    /// content language before comparison, so the match is symmetric.
    #[test]
    fn language_range_matches_canonicalizes_deprecated_primary_language_subtag() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            language_range_matches("he", "iw"),
            ":lang(he) must match the deprecated-but-still-in-the-wild lang=\"iw\""
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            language_range_matches("iw", "he"),
            ":lang(iw) must also match lang=\"he\" — the :lang() argument \
             itself is canonicalized too, per Selectors L4 §7.2"
        );
    }

    /// Regression coverage for the rest of [`DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS`]
    /// beyond the `iw`/`he` pair pinned above, including `bh` -> `bih` — the
    /// one entry where the `Preferred-Value` changes the subtag's *length*
    /// (2 letters -> 3), which is exactly where an off-by-one in the
    /// canonicalize-then-compare ordering would surface.
    #[test]
    fn language_range_matches_canonicalizes_remaining_deprecated_subtag_table_entries() {
        for (deprecated, preferred) in [
            ("bh", "bih"),
            ("in", "id"),
            ("ji", "yi"),
            ("jw", "jv"),
            ("mo", "ro"),
        ] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                language_range_matches(preferred, deprecated),
                ":lang({preferred}) must match lang=\"{deprecated}\""
            );
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                language_range_matches(deprecated, preferred),
                ":lang({deprecated}) must match lang=\"{preferred}\""
            );
        }
    }

    /// Regression pin: a well-formed, multi-subtag tag/range pair that does
    /// NOT involve any deprecated subtag must still match exactly as before
    /// this well-formedness/canonicalization pass — exercises a `script`
    /// subtag (`Hans`) through the new validation pipeline, distinct from
    /// the plain-primary-language pairs the pre-existing tests use.
    #[test]
    fn language_range_matches_well_formed_non_canonicalized_tag_still_matches() {
        assert!(language_range_matches("zh-Hans", "zh-Hans-CN"));
        assert!(!language_range_matches("zh-Hant", "zh-Hans-CN"));
    }

    /// RFC 5646 §2.1's `privateuse = "x" 1*("-" (1*8alphanum))` is a live,
    /// still-usable `Language-Tag` alternative on its own (distinct from the
    /// frozen `grandfathered` list) — a bare top-level `privateuse` tag like
    /// `x-foo` must be well-formed and must match itself.
    #[test]
    fn language_range_matches_privateuse_tag_matches_itself() {
        assert!(language_range_matches("x-foo", "x-foo"));
        // The ABNF's `1*` requires at least one value subtag after the `x`
        // introducer — a bare `x` alone is ill-formed and must not match.
        assert!(!language_range_matches("x", "x"));
    }

    /// `dir="auto"` resolves via the element's own contained-text scan
    /// (HTML LS §3.2.6.4's "auto directionality" — see
    /// [`resolve_directionality`]'s doc), *not* by falling through to the
    /// nearest ancestor's directionality — even when that ancestor has an
    /// explicit `dir` of its own. `background-color` (not inherited, unlike
    /// `font-family`) proves the span itself resolved `ltr` rather than
    /// merely failing to match `:dir(rtl)` while still visually inheriting
    /// the ancestor's rtl-associated styling.
    #[test]
    fn dir_auto_scans_own_text_ltr_first_strong_overrides_rtl_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // "Hello" is wrapped in an ordinary (non-excluded) nested <b>, not a
        // direct text child of `span` — exercises the recursive descent
        // into an un-excluded element subtree in
        // `auto_text_scan_subtree`, not just the direct-text-child case.
        let bold = doc.push_element(span, "b", None);
        doc.push_text(bold, "Hello"); // first strong character 'H' is type L

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[article].background_color, RED,
            ":dir(rtl) must still match the article's own explicit dir=\"rtl\""
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "dir=\"auto\" must resolve via the element's own text scan (first \
             strong character 'H' is type L → ltr), not by inheriting the \
             rtl ancestor's directionality"
        );
    }

    /// Mirror of the LTR-first-strong case above, with the ancestor now
    /// `ltr` and the `dir=\"auto\"` element's own text starting with an
    /// Arabic (bidirectional type AL) character — resolves `rtl` despite
    /// the `ltr` ancestor.
    #[test]
    fn dir_auto_with_arabic_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}"); // "السلام"

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is Arabic (type AL) → rtl"
        );
    }

    /// Same shape as the Arabic case, but with a Hebrew (bidirectional type
    /// R, the non-Arabic right-to-left branch of
    /// [`strong_bidi_type`]) first strong character.
    #[test]
    fn dir_auto_with_hebrew_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{05E9}\u{05DC}\u{05D5}\u{05DD}"); // "שלום"

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is Hebrew (type R) → rtl"
        );
    }

    /// [`strong_bidi_type`]'s `R_RANGES` table, Hebrew Presentation Forms
    /// case (U+FB1D..U+FB4F, distinct from the main Hebrew block
    /// U+0590..U+05FF the test above exercises) — a separate
    /// `DerivedBidiClass.txt` `@missing` range and therefore a separate
    /// table entry that needs its own coverage.
    #[test]
    fn dir_auto_with_hebrew_presentation_forms_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{FB1D}"); // HEBREW LETTER YOD WITH HIRIQ

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "Hebrew presentation forms character is type R → rtl"
        );
    }

    /// U+200E LEFT-TO-RIGHT MARK (`Cf`, General Punctuation block) is one of
    /// [`strong_bidi_type`]'s explicit top-of-function exceptions (type `L`,
    /// checked before either range table) — its own doc's history matters
    /// here: this exact case (U+200E as text's first strong character,
    /// followed by unrelated Arabic text) previously resolved wrongly
    /// (`rtl` instead of the spec-correct `ltr`) before that check existed,
    /// because `is_alphabetic('\u{200E}') == false` fell through to `None`.
    #[test]
    fn dir_auto_with_leading_ltr_mark_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+200E LEFT-TO-RIGHT MARK, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{200E}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "first strong character is U+200E (type L) → ltr, despite \
             unrelated Arabic text right after it"
        );
    }

    /// U+200F RIGHT-TO-LEFT MARK (`Cf`, General Punctuation block) is
    /// [`strong_bidi_type`]'s other explicit top-of-function exception
    /// (type `R`) — mirrors the U+200E case above with the roles reversed.
    #[test]
    fn dir_auto_with_leading_rtl_mark_before_latin_text_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+200F RIGHT-TO-LEFT MARK, then unrelated Latin (type L) text.
        doc.push_text(span, "\u{200F}Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is U+200F (type R) → rtl, despite \
             unrelated Latin text right after it"
        );
    }

    /// [`strong_bidi_type`]'s `NON_STRONG_ALPHABETIC_RANGES` table, `NSM`
    /// case: U+0941 DEVANAGARI VOWEL SIGN U is `Alphabetic=Yes`
    /// (`char::is_alphabetic() == true`) but its real `Bidi_Class` is
    /// `NSM` (non-strong), not `L`. Without the exclusion table this
    /// leading combining mark would be misclassified as strong `L` and
    /// stop the scan immediately, resolving `ltr`; the correct scan skips
    /// it and continues to the following Hebrew (type `R`) text, resolving
    /// `rtl`.
    #[test]
    fn dir_auto_skips_non_strong_alphabetic_combining_mark_before_hebrew_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0941 DEVANAGARI VOWEL SIGN U, then "שלום" (Hebrew, type R).
        doc.push_text(span, "\u{0941}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "leading non-strong combining mark must be skipped (not \
             misclassified as strong L) so the scan reaches the Hebrew \
             text and resolves rtl"
        );
    }

    /// Same table, `ON` case (a spacing modifier letter, not a combining
    /// mark): U+02C6 MODIFIER LETTER CIRCUMFLEX ACCENT is also
    /// `Alphabetic=Yes` but its real `Bidi_Class` is `ON`, not `L`.
    #[test]
    fn dir_auto_skips_non_strong_alphabetic_modifier_letter_before_arabic_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+02C6 MODIFIER LETTER CIRCUMFLEX ACCENT, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{02C6}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "leading non-strong alphabetic modifier letter must be skipped \
             so the scan reaches the Arabic text and resolves rtl"
        );
    }

    /// [`strong_bidi_type`]'s `NON_STRONG_WITHIN_AL_R_RANGES` table: U+0664
    /// ARABIC-INDIC DIGIT FOUR falls inside `AL_RANGES`'s main Arabic span
    /// (U+0600..U+07BF) but its real `Bidi_Class` is `AN` (Arabic Number,
    /// weak), not `AL`. Without this exclusion table the range-table
    /// lookup alone would over-classify it as strong `AL` and stop the
    /// scan there, wrongly resolving `rtl` — not merely stopping one code
    /// point early, but resolving the *opposite* direction from the
    /// following Latin (type `L`) text's true first-strong result.
    #[test]
    fn dir_auto_skips_arabic_indic_digit_before_latin_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0664 ARABIC-INDIC DIGIT FOUR, then Latin "H".
        doc.push_text(span, "\u{0664}H");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Arabic-Indic digit (weak AN, not strong AL) must be \
             skipped so the scan reaches the Latin text and resolves ltr"
        );
    }

    /// [`strong_bidi_type`]'s `STRONG_L_NON_ALPHABETIC_RANGES` table, `Nd`
    /// case: U+0966 DEVANAGARI DIGIT ZERO has real `Bidi_Class=L` but
    /// `is_alphabetic() == false` (it is a decimal digit, not a letter).
    /// Without this table the digit would fall through to `None`
    /// (non-strong) and the scan would skip past it, wrongly resolving via
    /// the later Arabic (type `AL`) text instead — the opposite direction
    /// from the spec, since the digit is the text's true first strong
    /// (`L`) character.
    #[test]
    fn dir_auto_with_devanagari_digit_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0966 DEVANAGARI DIGIT ZERO, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{0966}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Devanagari digit (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall through \
             to the later Arabic text"
        );
    }

    /// Same table, `Po` case: U+055A ARMENIAN APOSTROPHE has real
    /// `Bidi_Class=L` but `is_alphabetic() == false` (it is punctuation,
    /// not a letter). Mirrors the Devanagari-digit test above with a
    /// Hebrew (type `R`) character standing in for the later strong
    /// character that must NOT be reached.
    #[test]
    fn dir_auto_with_armenian_punctuation_before_hebrew_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+055A ARMENIAN APOSTROPHE, then "שלום" (Hebrew, type R).
        doc.push_text(span, "\u{055A}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Armenian punctuation (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall \
             through to the later Hebrew text"
        );
    }

    /// Same table, `Mc` case: U+1B44 BALINESE ADEG ADEG is a virama (a
    /// spacing combining mark that suppresses the inherent vowel of the
    /// preceding consonant) with real `Bidi_Class=L`, but Unicode does not
    /// consider a virama `Alphabetic` the way it considers an ordinary
    /// vowel sign alphabetic, so `is_alphabetic() == false` here too.
    /// Mirrors the two tests above with Arabic (type `AL`) standing in for
    /// the later strong character that must NOT be reached.
    #[test]
    fn dir_auto_with_balinese_virama_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+1B44 BALINESE ADEG ADEG, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{1B44}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Balinese virama (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall \
             through to the later Arabic text"
        );
    }

    /// Same table, `Mc` case, Sharada Vowel Signs Supplement (new in Unicode
    /// 17.0.0): U+11B61 SHARADA VOWEL SIGN OOE has real `Bidi_Class=L` and
    /// is `Alphabetic=Yes` per UCD 17.0.0, but this workspace's pinned rustc
    /// 1.89.0 predates that Unicode version, so `char::is_alphabetic()`
    /// still returns `false` for it. Without its own table entry (as
    /// opposed to the Balinese virama above, which is never `Alphabetic`
    /// under any Unicode version) the vowel sign would fall through to
    /// `None` exactly the way the `is_alphabetic()`-fallback gap this table
    /// exists to close would predict.
    #[test]
    fn dir_auto_with_sharada_vowel_sign_ooe_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+11B61 SHARADA VOWEL SIGN OOE, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{11B61}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Sharada vowel sign (strong L, despite not being \
             is_alphabetic() under this workspace's pinned rustc) must \
             resolve ltr on its own, not fall through to the later Arabic \
             text"
        );
    }

    #[test]
    fn strong_bidi_range_tables_preserve_classification_invariants() {
        fn overlaps(a: (u32, u32), b: (u32, u32)) -> bool {
            a.0 <= b.1 && b.0 <= a.1
        }

        fn assert_table_is_internally_disjoint(name: &str, table: &[(u32, u32)]) {
            for (index, &left) in table.iter().enumerate() {
                assert!(left.0 <= left.1, "{name}[{index}] has an inverted range");
                for (other_index, &right) in table.iter().enumerate().skip(index + 1) {
                    // cov:ignore: the failure-message branch of this `assert!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert!(
                        !overlaps(left, right),
                        "{name}[{index}] {left:#x?} overlaps {name}[{other_index}] {right:#x?}"
                    );
                }
            }
        }

        fn assert_tables_are_disjoint(
            left_name: &str,
            left: &[(u32, u32)],
            right_name: &str,
            right: &[(u32, u32)],
        ) {
            for (left_index, &left_range) in left.iter().enumerate() {
                for (right_index, &right_range) in right.iter().enumerate() {
                    // cov:ignore: the failure-message branch of this `assert!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert!(
                        !overlaps(left_range, right_range),
                        "{left_name}[{left_index}] {left_range:#x?} overlaps \
                         {right_name}[{right_index}] {right_range:#x?}"
                    );
                }
            }
        }

        let tables = [
            ("AL_RANGES", AL_RANGES),
            ("R_RANGES", R_RANGES),
            (
                "NON_STRONG_WITHIN_AL_R_RANGES",
                NON_STRONG_WITHIN_AL_R_RANGES,
            ),
            ("NON_STRONG_ALPHABETIC_RANGES", NON_STRONG_ALPHABETIC_RANGES),
            (
                "STRONG_L_NON_ALPHABETIC_RANGES",
                STRONG_L_NON_ALPHABETIC_RANGES,
            ),
        ];

        for &(name, table) in &tables {
            assert_table_is_internally_disjoint(name, table);
        }
        assert_tables_are_disjoint("AL_RANGES", AL_RANGES, "R_RANGES", R_RANGES);
        assert_tables_are_disjoint(
            "AL_RANGES",
            AL_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "R_RANGES",
            R_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "AL_RANGES",
            AL_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "R_RANGES",
            R_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );

        for &(lo, hi) in NON_STRONG_WITHIN_AL_R_RANGES {
            // cov:ignore: the failure-message branch of this `assert!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert!(
                AL_RANGES
                    .iter()
                    .chain(R_RANGES)
                    .any(|&outer| outer.0 <= lo && hi <= outer.1),
                "non-strong exception {lo:#x}..={hi:#x} is outside AL/R defaults"
            );
        }

        for &(lo, hi) in NON_STRONG_ALPHABETIC_RANGES {
            for cp in lo..=hi {
                let character = char::from_u32(cp).expect("range is a Unicode scalar");
                let resolved = strong_bidi_type(character);
                if character.is_alphabetic() {
                    // cov:ignore: the failure-message branch of this `assert_eq!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert_eq!(
                        resolved, None,
                        "alphabetic NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must be excluded from L"
                    );
                }
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    resolved != Some(StrongBidiType::L),
                    "NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must never resolve as L"
                );
            }
        }
        for &(lo, hi) in STRONG_L_NON_ALPHABETIC_RANGES {
            for cp in lo..=hi {
                let character = char::from_u32(cp).expect("range is a Unicode scalar");
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    !character.is_alphabetic(),
                    "STRONG_L_NON_ALPHABETIC_RANGES contains alphabetic U+{cp:04X}"
                );
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(character),
                    Some(StrongBidiType::L),
                    "STRONG_L_NON_ALPHABETIC_RANGES entry U+{cp:04X} must resolve as L"
                );
            }
        }

        for &(lo, hi) in AL_RANGES {
            for cp in lo..=hi {
                let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                    .iter()
                    .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
                {
                    None
                } else {
                    Some(StrongBidiType::Al)
                };
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                    expected,
                    "AL_RANGES classification changed for U+{cp:04X}"
                );
            }
        }
        for &(lo, hi) in R_RANGES {
            for cp in lo..=hi {
                let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                    .iter()
                    .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
                {
                    None
                } else {
                    Some(StrongBidiType::R)
                };
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                    expected,
                    "R_RANGES classification changed for U+{cp:04X}"
                );
            }
        }
    }

    /// No strongly-directional character anywhere in the `dir=\"auto\"`
    /// element's contained text (digits, spaces, and punctuation are all
    /// non-strong) — HTML LS's own `Auto`-state fallback is `'ltr'`
    /// unconditionally, never the ancestor's directionality (contrast with
    /// `dir_undefined_still_falls_through_to_ancestor_via_background_color`
    /// below, where an *absent* `dir` attribute does fall through to this
    /// same `rtl` ancestor).
    #[test]
    fn dir_auto_with_no_strong_directional_text_falls_back_to_ltr_despite_rtl_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "123 456!");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "no strong L/AL/R character anywhere → 'ltr' fallback, not the \
             rtl ancestor's directionality"
        );
    }

    /// A missing `dir` attribute (HTML LS "Undefined" state) is a
    /// *different* case from `dir=\"auto\"` above and must keep falling
    /// through to the ancestor's directionality — `background-color`
    /// isolates the element's own `:dir()` match from ordinary (unrelated)
    /// property inheritance the same way the tests above do.
    #[test]
    fn dir_undefined_still_falls_through_to_ancestor_via_background_color() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None); // no dir attribute at all

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "missing dir attribute must still fall through to the ancestor's \
             directionality"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s `dir`-attribute-state branch: a
    /// descendant with its own non-`Undefined` `dir` (here `rtl`) resolves
    /// its *own* directionality independently and must not leak its text
    /// into an ancestor's auto-directionality scan. Tree order would visit
    /// the nested Arabic text before the outer element's own trailing
    /// English text — if the exclusion were missing, the scan would
    /// (wrongly) stop at the Arabic text and resolve `rtl`.
    #[test]
    fn auto_directionality_skips_descendant_with_own_dir_attribute() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let inner = doc.push_element(outer, "span", None);
        doc.set_attr(inner, "dir", "rtl");
        doc.push_text(inner, "\u{0627}"); // Arabic alef — must be skipped
        doc.push_text(outer, "Hello"); // outer's own trailing text

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "the dir=\"rtl\" descendant's text must be excluded from the \
             outer element's own auto-directionality scan"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s tag-name branch, `bdi` case,
    /// isolated from the `dir`-attribute branch above (this `bdi` element
    /// carries no `dir` attribute of its own — it is excluded purely by
    /// being a `bdi` element, per HTML LS's exclusion list).
    #[test]
    fn auto_directionality_skips_bdi_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let bdi = doc.push_element(outer, "bdi", None); // no dir attribute
        doc.push_text(bdi, "\u{0627}"); // Arabic alef — must be skipped
        doc.push_text(outer, "Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "a bdi descendant's text must be excluded from the outer \
             element's own auto-directionality scan, regardless of its own \
             dir state"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s tag-name branch, `script`/`style`
    /// case — stylesheet/script text must never be treated as page content
    /// for directionality purposes.
    #[test]
    fn auto_directionality_skips_script_and_style_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let nested_script = doc.push_element(outer, "script", None);
        doc.push_text(nested_script, "\u{0627}");
        let nested_style = doc.push_element(outer, "style", None);
        doc.push_text(nested_style, "\u{0627}");
        doc.push_text(outer, "Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "script/style descendant text must be excluded from the outer \
             element's own auto-directionality scan"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s namespace-gate branch: a
    /// foreign-namespace (SVG) descendant is not one of the 4 HTML-only
    /// excluded tag names (it cannot be, per HTML LS's own dfns), so its
    /// text must still be scanned like any ordinary descendant's — unlike
    /// the `bdi`/`script`/`style`/`textarea` cases above, this is a
    /// "must **not** be excluded" regression test.
    #[test]
    fn auto_directionality_scans_into_foreign_namespace_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let svg_text =
            doc.push_element_with_namespace(outer, "text", "http://www.w3.org/2000/svg", &[]);
        doc.push_text(svg_text, "\u{0627}"); // Arabic alef

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, RED,
            "a foreign-namespace descendant's text must still be scanned, \
             not excluded"
        );
    }

    /// Comment nodes are neither "text node" nor "element node" and must
    /// not contribute to the auto-directionality scan (mirrors
    /// [`matches_empty`] / [`is_substantial_node`]'s treatment of the same
    /// node kind). Deliberately has **no** other text anywhere in the
    /// element, so a wrong implementation that read the comment's own text
    /// content would resolve `rtl`, not merely fail to resolve `ltr` for an
    /// unrelated reason.
    #[test]
    fn auto_directionality_ignores_comment_node_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        doc.push_comment(outer, "\u{0627}"); // Arabic alef inside a comment

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "a comment node's text must never be scanned; with no other \
             text present this must resolve via the 'ltr' no-strong-\
             character fallback"
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
        // (`lang_pseudo_matches` doc) is enforced directly by this
        // function's own empty-`content_language` early return, which
        // requires exact string equality against `range` and so yields
        // `false` for a bare `*` range against an empty `content_language`
        // — this function itself just needs to accept any non-empty tag
        // for a bare `*` range (RFC 4647 §3.3.2 step 2's wildcard-subtag
        // clause), which is what the assertions below check.
        assert!(language_range_matches("*", "ja"));
        assert!(language_range_matches("*", "en-US"));
        assert!(language_range_matches("*", "und"));
    }

    /// Selectors L4 §7.2 (quoted in full on `lang_pseudo_matches`'s doc): "A
    /// language range consisting of an empty string (`:lang(\"\")`) matches
    /// (only) elements whose language is not tagged." An element with no
    /// `lang`/`xml:lang` anywhere in its ancestor chain is exactly that case
    /// — `effective_language`'s exhausted-ancestor-chain terminal case
    /// resolves to `String::new()` per HTML LS §3.2.6.2's own final fallback
    /// ("the corresponding language tag is the empty string").
    #[test]
    fn lang_empty_string_range_matches_when_no_lang_anywhere_in_ancestor_chain() {
        // `background-color`, not `font-family`: `font-family` is inherited
        // (CSS Fonts 4 §2), so a rule that only matched `html` or `body`
        // would still show up on `untagged` via ordinary inheritance,
        // making font-family unable to distinguish "matched `untagged`
        // itself" from "matched an ancestor and inherited down" (same
        // pitfall `root_pseudo_class_matches_the_document_root_element_only`'s
        // own comment documents). `background-color` is not inherited (CSS
        // Backgrounds 3 §2.2), so red on `untagged` can only mean
        // `:lang("")` matched `untagged` itself. `tagged` (an explicit
        // `lang="en"` sibling) is the negative control proving the rule
        // isn't matching unconditionally.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(\"\") { background-color: red }");
        let html = doc.push_element(0, "html", None); // no lang
        let body = doc.push_element(html, "body", None); // no lang
        let untagged = doc.push_element(body, "p", None); // no lang
        let tagged = doc.push_element(body, "p", None);
        doc.set_attr(tagged, "lang", "en");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[untagged].background_color, RED,
            ":lang(\"\") must match an element with no lang/xml:lang anywhere \
             in its ancestor chain, per Selectors L4 §7.2"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[tagged].background_color,
            ComputedValues::initial().background_color,
            ":lang(\"\") must not match an element with an explicit lang \
             attribute in its own chain"
        );
    }

    /// Companion regression pin for the previous test: `:lang(*)` must NOT
    /// match the same no-lang-anywhere element (Selectors L4 §7.2: "a
    /// wildcard language range (\"*\") does not match elements whose
    /// language is not tagged"). Distinct from the existing
    /// `language_range_matches_wildcard_matches_any_tagged_language` test,
    /// which exercises `language_range_matches` directly as a Rust
    /// function call — this one goes through the full CSS parse + cascade
    /// path, exercising `effective_language`'s terminal
    /// ancestor-chain-exhausted case directly, where the resolved content
    /// language is the empty string.
    ///
    /// The range must be **quoted** (`:lang("*")`, not bare `:lang(*)`) —
    /// `*` is a CSS delimiter token, not a valid `<ident>` character, so the
    /// unquoted form fails `expect_ident_or_string()`
    /// (`parse_non_ts_functional_pseudo_class`'s `:lang()` arm) and the
    /// whole rule is dropped as an invalid selector.
    #[test]
    fn lang_wildcard_range_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(\"*\") { font-family: wildcard-font }");
        let html = doc.push_element(0, "html", None); // no lang
        let body = doc.push_element(html, "body", None); // no lang
        let untagged = doc.push_element(body, "p", None); // no lang
        let tagged = doc.push_element(body, "p", None);
        doc.set_attr(tagged, "lang", "en");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[untagged].font_family,
            ComputedValues::initial().font_family,
            ":lang(*) must not match an element with no lang/xml:lang \
             anywhere in its ancestor chain, per Selectors L4 §7.2"
        );
        // Positive control: without this, a silently-dropped/unparsed
        // `:lang(*)` rule would make the negative assertion above pass
        // vacuously.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[tagged].font_family[0].to_string(),
            "wildcard-font",
            ":lang(*) must match an element with a real, non-empty lang \
             attribute"
        );
    }

    // --- structural pseudo-classes ---
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
        // Acceptance-pinning test for the correction documented in
        // `matches_empty` doc's "Spec provenance and correction" note:
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
        // Acceptance: `:nth-child(2n+1)` zebra
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
        // Acceptance: `.section > p:first-child { font-weight: bold }`.
        // Combines the child combinator with a
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
    fn nth_child_of_extended_syntax_is_accepted_as_selector_list_argument() {
        let list = crate::parse_selector_list("p:nth-child(2n+1 of .foo)")
            .expect("the `of S` selector-list syntax must parse");
        assert_eq!(list.slice().len(), 1);
    }

    #[test]
    fn nth_child_of_selector_list_filters_siblings_for_both_directions() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "li:nth-child(2 of .featured, [data-kind=selected]) { color: red } \
             li:nth-last-child(2 of .featured, [data-kind=selected]) { background-color: red }",
        );
        let ul = doc.push_element(0, "ul", None);

        let first_featured = doc.push_element(ul, "li", None);
        doc.set_attr(first_featured, "class", "featured");

        let unfiltered_before_second = doc.push_element(ul, "li", None);

        let second_filtered = doc.push_element(ul, "li", None);
        doc.set_attr(second_filtered, "data-kind", "selected");

        let third_filtered_non_li = doc.push_element(ul, "div", None);
        doc.set_attr(third_filtered_non_li, "class", "featured");

        let unfiltered = doc.push_element(ul, "li", None);

        let second_from_end = doc.push_element(ul, "li", None);
        doc.set_attr(second_from_end, "class", "featured");

        let unfiltered_before_last = doc.push_element(ul, "li", None);

        let last_filtered = doc.push_element(ul, "li", None);
        doc.set_attr(last_filtered, "data-kind", "selected");

        let tree = build_rule_tree(&doc);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            tree.style_rules.len(),
            2,
            "flat selector-list filters must remain captured"
        );
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(r.computed[second_filtered].color, RED);
        assert_eq!(r.computed[second_from_end].background_color, RED);
        assert_eq!(
            r.computed[first_featured].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[third_filtered_non_li].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[unfiltered].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[unfiltered_before_second].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[unfiltered_before_last].background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(
            r.computed[last_filtered].background_color,
            ComputedValues::initial().background_color
        );
    }

    #[test]
    fn nested_nth_child_filter_is_rejected_before_sibling_scan() {
        // Selectors L4 permits a complex-real-selector-list in `of S`, but this
        // implementation rejects nested structural nth components before the
        // outer filter can scan the sibling list. Keep enough siblings here to
        // make accidentally accepting the nested filter observable.
        let selector = "li:nth-child(1 of :nth-child(1))";
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            crate::parse_selector_list(selector).is_ok(),
            "the nested nth-child selector must parse before support filtering"
        );

        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "li:nth-child(1 of :nth-child(1)) { background-color: red }",
        );
        let list = doc.push_element(0, "ul", None);
        let sibling_count = 256;
        let items: Vec<_> = (0..sibling_count)
            .map(|_| doc.push_element(list, "li", None))
            .collect();

        let tree = build_rule_tree(&doc);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            tree.style_rules.is_empty(),
            "nested nth-child filters must be dropped before sibling scans"
        );
        let result = cascade(&doc, &tree).expect("cascade Ok");
        for item in items {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                result.computed[item].background_color,
                ComputedValues::initial().background_color,
                "nested nth-child filter must not style any of {sibling_count} siblings"
            );
        }
    }

    #[test]
    fn nested_nth_last_child_filter_is_rejected_before_sibling_scan() {
        // The from-end form must share the same bounded support boundary as
        // the from-start form; otherwise it could retain a second expensive
        // nested sibling scan path.
        let selector = "li:nth-last-child(1 of :nth-last-child(1))";
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            crate::parse_selector_list(selector).is_ok(),
            "the nested nth-last-child selector must parse before support filtering"
        );

        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "li:nth-last-child(1 of :nth-last-child(1)) { background-color: red }",
        );
        let list = doc.push_element(0, "ul", None);
        let sibling_count = 256;
        let items: Vec<_> = (0..sibling_count)
            .map(|_| doc.push_element(list, "li", None))
            .collect();

        let tree = build_rule_tree(&doc);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            tree.style_rules.is_empty(),
            "nested nth-last-child filters must be dropped before sibling scans"
        );
        let result = cascade(&doc, &tree).expect("cascade Ok");
        for item in items {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                result.computed[item].background_color,
                ComputedValues::initial().background_color,
                "nested nth-last-child filter must not style any of {sibling_count} siblings"
            );
        }
    }

    #[test]
    fn nth_child_of_ignores_inert_element_siblings() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "li:nth-child(2 of .featured) { color: red }");
        let ul = doc.push_element(0, "ul", None);

        let first_featured = doc.push_element(ul, "li", None);
        doc.set_attr(first_featured, "class", "featured");

        let inert_featured = doc.push_element(ul, "li", None);
        doc.set_attr(inert_featured, "class", "featured");
        doc.set_in_document(inert_featured, false);

        let second_featured = doc.push_element(ul, "li", None);
        doc.set_attr(second_featured, "class", "featured");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(r.computed[second_featured].color, RED);
        assert_eq!(
            r.computed[inert_featured].color,
            ComputedValues::initial().color
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
    /// higher than any selector") pin.
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
    /// 「今の幅を前提にした算術の pin」より頑丈 (hardcoded const assert では
    /// なく実測 test を採る設計)。
    ///
    /// # 未 cover: cascade 経由の end-to-end pin (本 test 執筆時点では実装不可だった)
    ///
    /// もう 1 つの選択肢 (`<p id class>` に対する高 specificity
    /// selector と inline style を実際に cascade させ、inline が勝つことを
    /// 見る e2e test) は本 test 執筆時点では構築できなかった: 当時の
    /// `ruletree.rs` `is_type_or_universal_only` が id/class を含む selector
    /// を rule tree 構築時点で drop し、当時の `match_by_tag` も
    /// type/universal 以外の component を持つ selector を一致させなかった
    /// (当時は type + universal selector のみ対応だった)。したがって本 test は
    /// 「numeric な不変条件そのもの」を `crate::parse_selector_list` 経由で // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 直接 pin するに留めた。
    ///
    /// class/id/attribute selector matching の実装後
    /// (`is_type_or_universal_only` → `is_supported_selector_list`
    /// rename、`match_by_tag` → `match_simple_selectors` rename +
    /// `StyleElement` 対応)、上記の e2e test を阻んでいたブロッカーは解消
    /// 済み。e2e test 自体の追加は本 test の scope 外のまま、将来の課題
    /// として残す。
    ///
    /// selector 内の class / pseudo-class 数も併せて増やし
    /// (class_like_selectors field)、id field 単独ではなく複数 field が
    /// 同時に飽和する構成にしている。element_selectors field (type selector /
    /// pseudo-element 由来) は 1 compound selector につき type selector を
    /// 1 つしか持てないが、descendant combinator で compound を連結すれば
    /// compound ごとに `Component::LocalName` が積み上がるため、この field も
    /// 公開 API 経由で飽和させられる (`RaikiriSelectorParser` は
    /// pseudo-element 未サポート — `crate::PseudoElem` は uninhabited — // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
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
             packed-specificity field 幅が変わった signal (cascade.rs \
             INLINE_SPECIFICITY doc 参照)。"
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
    /// 共有される。**この共有が持ち込む唯一の新しい
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

    /// `collect_cascaded` の出力が per-node
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
    /// (「global index space を渡すと壊れる」という既知のハザードの、
    /// arena 版の再発防止)。2 つの assertion で役割が分かれる:
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
        // (a known "global index space" hazard, arena-shaped).
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

    // ── 絶対化 (Phase 2) ───────────────────
    //
    // ここから下の test 群は「cascade を抜けた時点で length が px に解決されて
    // いる」ことを pin する。従来は specified value が
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

    /// Rationale 1 — **本 task の存在理由**。
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

    // ── `lh` / `rlh` (CSS Values 4 §6.1.1) ──────────

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
    /// 倒す (`crate::resolve::resolve_length_percentage` doc、cleanroom: // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 比率を捏造しない)。
    #[test]
    fn lh_falls_back_to_zero_when_own_line_height_is_normal() {
        let cv = cascade_doc("", "div", Some("padding: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    /// Regression pin: `margin-top: 1lh`
    /// under the extremely common `line-height: normal` configuration must
    /// compute to `Px(0.0)` — margin's true spec initial (CSS Box 3 §3.1) —
    /// **not** `Auto`. `Auto` would silently trigger real taffy auto-margin
    /// layout (space distribution / centering) with no spec basis, which is
    /// the concrete failure mode the fix (`resolve_margin_length_or_auto`)
    /// closes.
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
    /// `crate::resolve::resolve_line_height`) — it must use the **parent's** // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
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
    /// in this crate (`rlh`'s own definition, "the
    /// lh unit on the root element", is a tree-global constant that does not
    /// depend on the declaring element's position; see
    /// `crate::resolve::resolve_line_height`'s doc for why the literal // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
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

    // ── `font-size: 1lh` / `1rlh` (CSS Values 4 §6.1.1) ──

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
    /// share a *generic* resolver with a known `0px` compromise),
    /// `resolve_font_size` is a dedicated
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

    /// **D5 invariant**。
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

    /// **D5 invariant** — `font-size` 版 (`bolder` の
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
    /// (`crate::specified::SpecifiedValues::finalize_as_root` doc の // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
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

    /// `collect_cascaded` and `resolve_inheritance`
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
    /// O(`depth`) — this isolates the pure *stack-depth* question this test
    /// answers. A descendant-combinator version was tried
    /// first and rejected for an unrelated reason, not stack depth:
    /// `Combinator::Descendant`'s backtracking was
    /// separately exponential on uniformly-matching chains (discovered by
    /// that attempt) — since fixed by memoization, see
    /// [`match_combinator_chain`]'s "Memoization" doc and
    /// `descendant_combinator_deep_unsatisfiable_chain_does_not_explode`
    /// below for that fix and its regression test.
    ///
    /// Before this fix, this forced the
    /// `match_combinator_chain`/`match_from_element` mutual recursion
    /// (renamed from `match_from_ancestor`) to a
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

    /// Empirical answer to the question: does a long, uniformly-
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
    /// hypothetical, overflow risk. After converting the recursion to the
    /// explicit-stack form
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

    // ── UA origin + display cascade ──

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

        // build_rule_tree (Author 集約) + UA add_stylesheet
        let mut tree = build_rule_tree(&doc);
        // UA CSS を先頭に inject するのではなく、既存の Author rule の後ろに
        // add してから rank 化で origin 順序を担保する (source_order より rank
        // が優位)
        // ただし現状 add_stylesheet の呼び出し順で source_order が振られ
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
    fn author_display_list_item_computes_through_cascade() {
        // End-to-end pipeline pin (parse -> cascade -> ComputedValues) for
        // `display: list-item` — keyword-acceptance only, sibling of
        // `author_display_flex_and_grid_compute_through_cascade` above.
        let cv = cascade_with_ua("", "li { display: list-item }", "li", None);
        assert_eq!(cv.display, DisplayValue::ListItem);
    }

    #[test]
    fn author_display_contents_computes_through_cascade() {
        // CSS Display 3 §2.5 `contents` — same end-to-end pipeline pin as
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
        // End-to-end pipeline pin (parse -> cascade -> ComputedValues) for
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

    // ── cascade_rank 4-tier ordering (3rd tier: CSS
    // Cascading L5 §6.5 "author presentational hint origin"; 4th tier:
    // CSS Cascading L4 §6.2 "user origin") ──

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
        // `important = false`), so `(AuthorPresentationalHint, true)`
        // stays production-unreached. `(User, false)` / `(User, true)`
        // used to be production-unreached too (no code path routed any
        // declaration to `Origin::User`) until consumer `extra_stylesheets`
        // was wired to it — this test remains the only
        // place `(AuthorPresentationalHint, true)` is exercised, but the
        // two `User` arms now also have real end-to-end coverage via
        // `crates/raikiri/tests/build_cascaded.rs`'s
        // `extra_stylesheets_user_rule_overrides_ua_via_umbrella` (normal)
        // and `user_important_beats_normal_ua_via_umbrella` (important).
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

    // ── background-color wire-through (CSS Backgrounds 3 §2.2) ──

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
        // per-node に到達することを pin (parse_color の transparent Ident branch
        // と CssColor::TRANSPARENT const の regression canary)。
        let cv = cascade_doc("", "div", Some("background-color: transparent"));
        assert_eq!(cv.background_color, CssColor::TRANSPARENT);
        assert_eq!(cv.background_color.a, 0);
    }

    // ── line-height wire-through (CSS Inline 3 §5.1) ──

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

    // ── counter-* wire-through (CSS Lists 3 §4、将来の GCPM 対応に向けた足場) ──

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

    // ── quotes wire-through (CSS Content 3 §2.4.1) ──

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

    // ── content wire-through (CSS Content 3 §2) ──

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

    // ── string-set wire-through (CSS GCPM 3 §1.1.1) ──

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

    // ── position: running() wire-through (CSS GCPM 3 §1.2.1) ──

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

    // ── text-align wire-through + inheritance (CSS Text 3 §6.1) ──

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

    // ── text-indent wire-through + inheritance (CSS Text 3 §8.1) ──

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
        // "re-resolve しない" 性質を子の font-size を変えて pin する)。
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

    // ── direction wire-through (CSS Writing Modes 4 §2.1) ──

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

    // ── font-style wire-through (CSS Fonts 4 §2.4) ──

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

    // ── font-variant-caps wire-through (CSS Fonts Module Level 3 §6.6) ──

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

    // ── text-transform wire-through (CSS Text Module Level 3 §2.1) ──

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

    // ── visibility wire-through (CSS Display 3 §4) ──

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

    // ── word-break wire-through (CSS Text 3 §5.1) ──

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

    // ── overflow-wrap / word-wrap legacy alias wire-through (CSS Text 3 §5.4) ──

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

    // ── letter-spacing / word-spacing wire-through (CSS Text 3 §7.2 / §7.1) ──

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

    /// Inheritance carries the parent's already-**computed** length, not the
    /// specified `em` re-resolved against the child's own font-size (CSS
    /// Cascade 5 §7.2 "Inheritance": inherited values are the parent's
    /// computed values; CSS Text 3 §7.2 "Computed value: an absolute
    /// length"). A px-only fixture (the sibling tests above) can't
    /// distinguish "inherit the computed px" from "inherit the specified
    /// `em`/`px` and re-resolve" — those only diverge when the child's
    /// font-size differs from the parent's, which requires an `em` value.
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

    // ── tab-size wire-through (CSS Text Module Level 3 §4.2) ──

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

    /// Inheritance carries the parent's already-**computed** length, not the
    /// specified `em` re-resolved against the child's own font-size — same
    /// shape as
    /// `letter_spacing_inherited_em_value_does_not_re_resolve_against_child_font_size`
    /// above (CSS Cascade 5 §7.2 "Inheritance": inherited values are the
    /// parent's computed values; CSS Text Module Level 3 §4.2 "Computed
    /// value: the specified number or absolute length").
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

    // ── text-shadow wire-through (CSS Text Decoration Module Level 3 §4) ──

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

    // ── border-radius / box-shadow / outline wire-through ────────────────

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
                top_left: ComputedLength(16.0),
                top_right: ComputedLength(32.0),
                bottom_right: ComputedLength(48.0),
                bottom_left: ComputedLength(64.0),
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
                },
                ComputedBoxShadowItem {
                    offset_x: ComputedLength(2.0),
                    offset_y: ComputedLength(3.0),
                    blur_radius: ComputedLength::ZERO,
                    spread_radius: ComputedLength::ZERO,
                    color: TextShadowColor::CurrentColor,
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

    /// CSS Basic User Interface Module Level 3 §4.3: `outline-style: auto` is
    /// an outline-only keyword. It must survive cascade/computed propagation
    /// without changing the border style type or parser behavior.
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

    /// Inheritance carries the parent's already-**computed** lengths, not the
    /// specified `em` re-resolved against the child's own font-size — same
    /// shape as `letter_spacing_inherited_em_value_does_not_re_resolve_against_child_font_size`.
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

    // ── white-space wire-through (CSS Text 3 §3) ──

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

    // ── hyphens wire-through (CSS Text 3 §5.3) ──

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

    // ── orphans / widows wire-through (CSS Fragmentation Module Level 3
    //    §3.3) ──

    // ── writing-mode wire-through (CSS Writing Modes 4 §3.2) ──

    #[test]
    fn writing_mode_wired_through_cascade_from_inline_style() {
        use crate::property::WritingMode;
        let cv = cascade_doc("", "p", Some("writing-mode: vertical-rl"));
        // `apply_value`'s `WritingMode` arm assigns the raw specified
        // keyword; the `HorizontalTb` collapse for non-horizontal keywords
        // happens later in `absolutize_with` (see `apply_value`'s
        // `PropertyValue::WritingMode` arm doc comment), so the computed
        // value here is always `HorizontalTb` even for `vertical-rl`.
        assert_eq!(cv.writing_mode, WritingMode::HorizontalTb);
    }

    // ── background-repeat/attachment/clip/origin/size/position wire-through
    // (CSS Backgrounds and Borders 3 §2.4-§2.9) ──

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
        // `background-clip`'s (`border-box`) — pin both distinctly.
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

    // ── text-align: match-parent (CSS Text 3 §6.1) ──
    //
    // Before
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

    // ── box-sizing wire-through (CSS Sizing 3 §3.3) ──

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

    // ── vertical-align wire-through (CSS 2.1 §10.8.1) ──

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

    // ── z-index wire-through (CSS2 §9.9.1) ──

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

    // ── break-before / break-after / break-inside wire-through
    // (CSS Fragmentation Module Level 3 §3.1 / §3.2 / §3.4) ──

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
        // shorthand" section) — end-to-end pin that the non-identity remap
        // survives the full parse -> cascade -> ComputedValues pipeline, not
        // just the `property::tests` parser-level pin.
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("page-break-before: always"));
        assert_eq!(cv.break_before, BreakBetween::Page);
    }

    // ── font-weight keyword + inheritance (CSS Fonts 4 §2.2) ──

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

    // ── font-weight bolder / lighter (CSS Fonts 4 §2.2) ──

    /// `<p style="font-weight: {parent}"><span style="font-weight: {child}">` を
    /// cascade して span の computed font-weight を返す。
    ///
    /// **親の computed value** を経由することが本 helper の主眼 — child は
    /// literal な spec 値ではなく、親が cascade を通して確定させた weight に
    /// 対して relative resolution される (Verification #7)。
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

    /// `resolve_relative_weight` の doc 「非有限 `inherited` — 本関数は
    /// guard しない」節が記述する非対称処理を pin する。
    ///
    /// 「guard は sink 境界に置く、resolve
    /// 層には置かない」という既存方針に従い、**本関数自体は変更しない** — 非対称は
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
    /// (`font_weight_bolder_lighter_table_all_six_rows` の font-size 版)。
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
    /// いないことの pin。`as_property_value` (覗き見)
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

        // `background-position` — added `v @ (...)` pass-through arm (CSS
        // Backgrounds 3 §2.6). This arm is only reachable via the `@page`
        // path (`crate::page::cascade_page`) in practice — the element
        // path goes through `apply_value` directly — so this direct call
        // is this arm's only coverage of the 6 new `Background*` variants
        // (same shape as the `Color` assertion above, which covers
        // `background_color`'s sibling arm the same way).
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
        // computed side で pin する (parse 側の同値 pin は
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

    // ── Cascade memory DoS regression (SEC HIGH) ──
    //
    // 従来 `PropertyValue::Content(Vec<ContentComponent>)` /
    // `ComputedValues.content: Vec<ContentComponent>` は cascade 段の `decl.value.clone()`、
    // `apply_winners` の drain の `value.clone()`、`resolve_inheritance` の stack push + write と
    // 段階ごとに deep-clone を経由し、`* { content: "<large>" }` × N element で
    // O(N × M) 相当の heap 消費を招いていた。この修正で outer `Vec` を
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
             (SEC HIGH DoS regression)"
        );
    }

    /// `string-set` も content と同じ cascade path を辿るため、同種 Arc 共有が
    /// 成立している必要がある (同じ修正で `PropertyValue::StringSet` も Arc wrap)。
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

    // ── Cascade memory DoS regression (SEC HIGH) ──
    //
    // `counter-reset` /
    // `counter-increment` / `counter-set` は content/string-set と同じ
    // 3 段 clone 経路 (`decl.value.clone`、`apply_winners` の drain の `value.clone`、
    // `resolve_inheritance` の stack push + write) を辿るため
    // `PropertyValue::Counter*(Vec<..>)` × universal selector × N element で
    // O(N × M) 相当の heap 消費を招いていた。この修正で全 3 property の outer
    // `Vec` を `Arc<Vec<(SmolStr, i32)>>` に wrap、clone 経路が Arc bump に
    // 落ちた (asymptotic は O(N + M))。short-circuit (child stack entry で
    // counter-* を skip) は **意図的に採用せず** — content/string-set の修正も
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
             (SEC HIGH DoS regression)"
        );
    }

    /// `counter-increment` も counter-reset と同じ cascade path を辿るため、
    /// 同種 Arc 共有が成立している必要がある (同じ修正)。
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

    /// `counter-set` も同じ cascade path を辿るため Arc 共有が必要 (同じ修正)。
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

    /// counter-* は non-inherited — 親 element に counter 値があっても child は
    /// inherit_from で shared empty Arc slot に落ちる。この pin が「Arc wrap 単独
    /// (short-circuit 無し) でも child stack entry の parent Arc bump が即 empty
    /// slot に置換される」ことを保証する。short-circuit 不採用の正当化
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
        // どちらも上記の memory 目標を破る。
        let shared = crate::property::empty_counter_entries();
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[child].counter_reset, &shared),
            "child counter_reset must point to shared empty Arc slot \
             (non-inherited short-circuit-equivalent canary)"
        );
    }

    /// Empty (initial / inherit_from) の content/string_set も **shared Arc slot**
    /// を再利用する — cascade fix の副作用で「per-node empty Arc allocation
    /// regression」に陥っていないことを pin する。
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

    // ── font-family per-node malloc regression ──
    //
    // pre-existing finding (out-of-diff, unrelated to this change):
    // `ComputedValues.font_family` は本 crate で最後に
    // Arc-share されていなかった `Vec` payload (前述の修正群が Content/StringSet
    // と counter-* を対応済み)。あちら (non-inherited) と違い
    // `font-family` は **inherited** なので、支配的な per-node cost は
    // 「毎 node で initial にリセットする」コストではなく inheritance walk
    // (`SpecifiedValues::inherit_from` の `parent.font_family.clone()`) の
    // コスト — 同じ `Arc` fix でもコストの形が違う。上記と同じ
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
    /// になってしまう (この test の実装時に perturbation で確認済み —
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
             slot across independent cascade() runs"
        );
    }

    /// `* { font-family: ... }` × N element で、matching 全 element の
    /// `Vec` は勝者 declaration の Arc を共有しなければならない — mirrors
    /// `cascade_shares_content_arc_across_universal_selector_matches`。
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

    /// `font-family` declaration を持たない child は、親と**同一**の Arc を
    /// (`Arc::ptr_eq`) 継承しなければならない — 中身を再 clone したものでは
    /// ならない。これはこの (inherited) field の支配的コストである
    /// inheritance-walk cost そのものであり、前述の
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
             Arc by identity, not a re-cloned Vec"
        );
    }

    // ── padding wire-through (CSS Box 3 §4.1 + §4.2) ──
    //
    // Verification #7 (cascade wire-through + non-inheritance):
    // <div style="padding: 10px 5%"> の cascade 結果が populate、initial value 0
    // が inheritance walk で child に伝播しない (padding は non-inherited)。

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
        // 逆順で pin: `padding-top: 5px; padding: 10px;`
        // → 全 side = 10px (後段 shorthand が top も含めて上書き)。
        let cv = cascade_doc("", "div", Some("padding-top: 5px; padding: 10px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    // ── margin longhand + shorthand cascade (CSS Box 3 §3.1/§3.2) ──

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
    fn var_in_margin_shorthand_preserves_later_longhand_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin: var(--space); margin-left: 20px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(20.0));
    }

    #[test]
    fn var_in_outline_shorthand_projects_each_deferred_longhand() {
        // `outline` expands before cascade; after the custom property is
        // substituted, each deferred longhand must project its component from
        // the reparsed `Outline` shorthand rather than being dropped.
        let cv = cascade_doc(
            "",
            "div",
            Some("--outline: auto 2px red; outline: var(--outline)"),
        );
        assert_eq!(cv.outline.width(), ComputedLength(2.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Auto);
        assert_eq!(cv.outline.color, OutlineColor::Resolved(RED));
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

    // ── CSS Logical Properties and Values 1 §4.2/§4.4 margin-inline-*/
    //    margin-block-*/padding-inline-*/padding-block-* longhand +
    //    margin-inline/margin-block/padding-inline/padding-block shorthand
    //    wire-through ──
    //
    // raikiri は縦書きレンダリングパイプライン未実装のため writing-mode を
    // 常に HorizontalTb に潰し、かつ inline axis は `direction: ltr` を
    // 仮定した固定物理写像
    // (`PropertyValue::PaddingInline` doc 参照) — margin-inline-start/end →
    // margin-left/margin-right、margin-block-start/end →
    // margin-top/margin-bottom (padding も同型)。

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
    fn var_in_margin_inline_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_shorthand_preserves_later_longhand_cascade`
        // — `margin-inline` only fans out to `margin-left`/`margin-right`
        // (unlike `margin`'s 4-side fan-out), so the untouched block axis
        // must stay at initial (0), not at the `var()`-substituted value.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin-inline: var(--space); margin-left: 20px"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_margin_block_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for the block-axis 2-value shorthand — exercises
        // `crate::rule::expand_deferred`'s `MarginBlock` arm (only the
        // inline-axis sibling was previously covered by an end-to-end
        // var() test).
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin-block: var(--space); margin-top: 20px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_padding_inline_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for `padding-inline` — exercises `crate::rule::expand_deferred`'s
        // `PaddingInline` arm.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; padding-inline: var(--space); padding-left: 20px"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(20.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn var_in_padding_block_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for `padding-block` — exercises `crate::rule::expand_deferred`'s
        // `PaddingBlock` arm, the last of the 4 logical 2-value shorthands.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; padding-block: var(--space); padding-top: 20px"),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(20.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn var_in_padding_inline_start_longhand_preserves_cascade() {
        // `property_key_for_name`'s `"padding-inline-start" =>
        // PropertyKey::PaddingLeft` arm is only exercised by the deferred
        // (`var()`) path when `deferred.property` gets re-parsed by
        // `resolve_deferred_value` — this pins that round-trip for a single
        // logical longhand (the `margin-inline`/`padding-inline` shorthand
        // var() round-trip is covered by
        // `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // above).
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 7px; padding-inline-start: var(--space)"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(7.0));
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

    // ── height wire-through (CSS Sizing 3 §3.1.1) ──

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
        // まま。parser 側 pin (`height_rejects_negative_length`) と complementary
        // な end-to-end 挙動を確認。
        let cv = cascade_doc("", "div", Some("height: -10px"));
        assert_eq!(
            cv.height,
            ComputedLengthPercentageOrAuto::Auto,
            "negative height declaration must be dropped; height stays at initial"
        );
    }

    // ── <img width>/<img height> presentational hint
    // (HTML LS https://html.spec.whatwg.org/multipage/rendering.html#dimRendering) ──

    #[test]
    fn img_width_and_height_attributes_promoted_to_computed_style() {
        // Acceptance: `<img width="100"
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
        // "非負整数" とだけ要約すると誤解を招くが、
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
        // string case: `TestDoc`'s own `TestElementRef::attr()` override
        // normalises `""` to `None` as a simplification local to that mock
        // — unlike the real `ElementRef::attr()` (`raikiri-dom::dom_impl`),
        // which returns `Some("")` for a present-but-empty attribute (see
        // `StyleElement::attr`'s trait doc). Against `TestDoc`,
        // `push_img_dimension_hints` never even calls
        // `parse_html_dimension_value` for `width=""`, so the `Auto` result
        // above is `TestDoc`-only "attribute absent" behavior here, not
        // (only) a parse-failure outcome. Against the real DOM the same
        // `Auto` result still holds, but for a different reason:
        // `attr("width")` returns `Some("")`, and
        // `parse_html_dimension_value("")` itself rejects the empty string
        // at its first-digit check (both DOM implementations agree on the
        // end result here, just not on why).
        let node = doc
            .node(StyleNodeId::new(empty as u64))
            .expect("node exists");
        let elem = node.as_element().expect("element node");
        assert_eq!(elem.attr("width"), None);
    }

    #[test]
    fn non_img_element_width_height_attributes_not_promoted() {
        // scope narrowing: mapping は `img` のみ。
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
        // Cascade-origin pin: presentational hint は
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
        // Origin-rank pin (`push_img_dimension_hints`
        // doc's "Cascade origin" section): the hint is
        // `Origin::AuthorPresentationalHint` (rank below `Origin::Author`
        // per `cascade_rank`), while this `* { width: 30px }` rule is a
        // real `Origin::Author` rule with zero specificity (universal
        // selector). Before this, both sides shared
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
        // (旧名 `match_by_tag`、その後
        // `match_simple_selectors` → `compound_matches`/
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
        // 経由で `TestElementRef::attr()` の override (`crate::test_dom`
        // に追加済み) を叩く。`StyleElement::attr` の
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

    /// Sibling of `apply_value_direct_margin_shorthand_fall_through` for the
    /// CSS Logical Properties and Values 1 §4.2 `margin-inline` 2-value
    /// shorthand — same "not a safety net" rationale (`crate::property::PropertyValue::MarginInline` doc, // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// which is the canonical record). `start`/`end` deliberately differ so
    /// a `start`/`end` field swap in the `apply_value` arm would fail this
    /// test.
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

    /// Sibling of `apply_value_direct_margin_inline_shorthand_fall_through`
    /// for the block-axis `margin-block` shorthand.
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

    /// Sibling of `apply_value_direct_margin_inline_shorthand_fall_through`
    /// for the CSS Logical Properties and Values 1 §4.4 `padding-inline`
    /// 2-value shorthand.
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

    /// Sibling of `apply_value_direct_padding_inline_shorthand_fall_through`
    /// for the block-axis `padding-block` shorthand.
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
        };
        apply_value(PropertyValue::TextDecoration(shorthand), &mut cv);
        assert_eq!(cv.text_decoration_line, shorthand.line);
        assert_eq!(cv.text_decoration_style, shorthand.style);
        assert_eq!(cv.text_decoration_color, shorthand.color);
    }

    // ── border longhand + shorthand cascade ──

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

    // ── text-decoration longhand + shorthand cascade (CSS Text Decoration
    // Module Level 3 §2.1-§2.4) ──

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
        // `crate::rule::tests::text_decoration_shorthand_always_overwrites_all_three_longhand` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — rustdoc-blind, confirmed via わざと壊して確かめる
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
        // determinism regression (acceptance criteria for this stage of work)
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

    // ── post-parse shorthand injection ──────────────
    //
    // `add_stylesheet` の**後**に declaration を shorthand variant へ書き戻す
    // post-parse mutation 経路 — `crate::rule::parse_declaration_block` の
    // parse-time 展開はこの経路を守らない (`declaration_block_never_emits_
    // shorthand_keys` は parse 出口しか見ない)。この
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

    // ── HTML LS §15.3.9 "Margin collapsing quirks"
    // (push_margin_collapsing_quirk_declarations) ──
    //
    // These tests build a UA stylesheet mimicking minimal.css's real
    // `blockquote, figure, listing, p, plaintext, pre, xmp { margin-top:
    // 1em; margin-bottom: 1em; }` default-margin rule (using an
    // arbitrary-but-nonzero 16px so a zeroed vs. unaffected side is always
    // unambiguous), via `RuleTree::empty()` + `add_stylesheet(_,
    // Origin::UserAgent)` directly rather than `build_rule_tree` (which
    // only ever tags DOM `<style>` elements as `Origin::Author` —
    // `build_rule_tree_produces_author_origin_for_dom_style_elements` in
    // `ruletree.rs` pins that).

    fn margin_quirk_ua_tree(css: &str) -> RuleTree {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::UserAgent);
        tree
    }

    #[test]
    fn margin_collapsing_quirk_zeroes_start_for_first_child_of_body() {
        // `<body><p>text</p></body>`, quirks mode: `p` is the first
        // (only) child of `body` and has no substantial previous
        // siblings → rule 1 zeroes margin-top. `p` is not blank (has a
        // substantial text child), so rule 2 does not also zero
        // margin-bottom.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_zeroes_both_sides_when_blank() {
        // `<body><p></p></body>` — `p` is additionally blank (no
        // substantial children at all) → rule 2 also zeroes
        // margin-bottom.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element(body, "p", None);

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_does_not_apply_outside_quirks_mode() {
        // Same structure as the first-child test above, but each of the
        // two non-`Quirks` `StyleQuirksMode` states — the quirk is gated
        // on full quirks mode only. `LimitedQuirks` gets its own case
        // (not folded into `NoQuirks`) because it's a DOM Standard dfn
        // distinct from full quirks mode
        // (`crate::style_dom::StyleQuirksMode`'s own doc draws the same
        // distinction, and `class_selector_case_sensitivity_across_quirks_modes`
        // above pins the equivalent distinction for selector matching) —
        // a broken gate that folded `LimitedQuirks` in with `Quirks`
        // would not be caught by testing `NoQuirks` alone.
        for mode in [StyleQuirksMode::NoQuirks, StyleQuirksMode::LimitedQuirks] {
            let mut doc = TestDoc::new();
            doc.quirks_mode = mode;
            let body = doc.push_element(0, "body", None);
            let p = doc.push_element(body, "p", None);
            doc.push_text(p, "text");

            let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
            let r = cascade(&doc, &tree).expect("cascade Ok");
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                r.computed[p].margin.top,
                ComputedLengthPercentageOrAuto::Px(16.0),
                "{mode:?} must not trigger margin-collapsing-quirks zeroing"
            );
        }
    }

    #[test]
    fn margin_collapsing_quirk_substantial_text_sibling_blocks_zeroing() {
        // `<body>Hello<p>text</p></body>` — `p` IS CSS's `:first-child`
        // (no earlier *element* sibling), but HTML LS denies it "no
        // substantial previous siblings" because the text node "Hello" is
        // substantial (non-inter-element-whitespace). A selector-based UA
        // rule built on `:first-child` would zero this incorrectly; the
        // dedicated structural predicate must not.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_text(body, "Hello");
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a substantial (non-whitespace) previous text sibling must block zeroing"
        );
    }

    #[test]
    fn margin_collapsing_quirk_whitespace_only_previous_sibling_does_not_block_zeroing() {
        // `<body>   <p>text</p></body>` — the leading text node is
        // whitespace-only (inter-element whitespace per HTML LS §3.2.5),
        // so it is not substantial and must not block "no substantial
        // previous siblings".
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_text(body, "   \t\n");
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_comment_previous_sibling_does_not_block_zeroing() {
        // `<body><!--c--><p>text</p></body>` — comment nodes are never
        // substantial (HTML LS §15.3.9's "substantial" dfn only counts
        // text/element nodes).
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_comment(body, "c");
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_rule3_zeroes_start_for_blank_last_child_of_td() {
        // `<td>Hello<pre></pre></td>` — `pre` has a substantial previous
        // sibling ("Hello", rules 1/2 do not apply) but is the last
        // (only trailing) child of `td` and is blank → rule 3 zeroes
        // margin-top. `pre` (not `p`) is used deliberately so rule 4
        // (which only ever fires for `p`) cannot also zero margin-bottom
        // here, keeping rule 3 isolated.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let td = doc.push_element(0, "td", None);
        doc.push_text(td, "Hello");
        let pre = doc.push_element(td, "pre", None);

        let tree = margin_quirk_ua_tree("pre { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[pre].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[pre].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "rule 3 only zeroes margin-block-start, not margin-block-end"
        );
    }

    #[test]
    fn margin_collapsing_quirk_rule4_zeroes_end_for_p_last_child_of_td_even_when_not_blank() {
        // `<td>Hello<p>text</p></td>` — `p` has a substantial previous
        // sibling (rules 1/2 do not apply) and is not blank (rule 3 does
        // not apply either, it requires blank), but is `p` specifically
        // and has no substantial following sibling → rule 4 zeroes
        // margin-bottom regardless of blankness.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let td = doc.push_element(0, "td", None);
        doc.push_text(td, "Hello");
        let p = doc.push_element(td, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "no rule zeroes margin-top here"
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_th_parent_triggers_rule4_same_as_td() {
        // Same fixture as
        // `margin_collapsing_quirk_rule4_zeroes_end_for_p_last_child_of_td_even_when_not_blank`
        // but with a `th` container instead of `td` — every other test in
        // this group uses `td` for the `td`/`th`-only rules (3 and 4), so
        // this pins that the `th` half of that `is_td_or_th` check is
        // exercised too.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let th = doc.push_element(0, "th", None);
        doc.push_text(th, "Hello");
        let p = doc.push_element(th, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_container_that_is_not_body_td_th_does_not_gate_at_all() {
        // `<div><p>text</p></div>` — `p` is the first (only) child of a
        // `div`, which is none of `body`/`td`/`th`, so none of the four
        // rules can apply (rules 1/2 require a `body`/`td`/`th` parent;
        // rules 3/4 require `td`/`th` specifically) — the
        // `!is_body && !is_td_or_th` early-return path.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let div = doc.push_element(0, "div", None);
        let p = doc.push_element(div, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a div container (neither body nor td/th) must never trigger any \
             margin-collapsing-quirks rule"
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_rules_3_and_4_are_td_th_only_not_body() {
        // `<body>Hello<p></p></body>` — `p` is blank and has no
        // substantial following sibling, which would trigger rule 3 (if
        // blank+trailing were enough regardless of parent) or rule 4 (if
        // it applied to any parent) — but rules 3/4 are gated on a
        // `td`/`th` parent specifically, and `body` must not qualify.
        // Rule 1/2 also don't apply here (substantial previous sibling
        // "Hello"), so nothing should be zeroed.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_text(body, "Hello");
        let p = doc.push_element(body, "p", None);

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_author_declaration_still_overrides_zeroing() {
        // The spec frames this as a real UA-origin stylesheet rule
        // participating in normal cascade, not an unconditional override
        // — a real author declaration (any specificity) must still be
        // able to give the element a nonzero margin.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "p { margin-top: 16px; margin-bottom: 16px; }",
            Origin::UserAgent,
        );
        tree.add_stylesheet("p { margin-top: 5px; }", Origin::Author);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(5.0),
            "a real author declaration must beat the quirks zeroing regardless of \
             its specificity, since Origin::Author always outranks Origin::UserAgent"
        );
    }

    #[test]
    fn margin_collapsing_quirk_does_not_apply_to_elements_outside_default_margin_list() {
        // `<body><div>text</div></body>` — `div` is not one of HTML LS
        // §15.3.9's 17 "elements with default margins", so the quirk
        // must never fire for it even though it would otherwise satisfy
        // every structural condition rule 1 checks.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let div = doc.push_element(body, "div", None);
        doc.push_text(div, "text");

        let tree = margin_quirk_ua_tree("div { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[div].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "div is not an \"element with default margins\" — must be unaffected"
        );
    }

    #[test]
    fn margin_collapsing_quirk_foreign_namespace_element_is_not_gated() {
        // Same shape as `push_img_dimension_hints`'s own namespace-gate
        // regression (`foreign_namespace_img_local_name_does_not_get_the_hint`
        // above) — a foreign-namespace element that merely shares the
        // local name "p" must not pick up the quirks zeroing, even though
        // it otherwise satisfies every structural condition rule 1
        // checks (first child of `body`, no substantial previous
        // sibling).
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element_with_namespace(body, "p", "http://example.com/not-html", &[]);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a foreign-namespace element sharing the local name \"p\" must not \
             get the quirks zeroing"
        );
    }

    #[test]
    fn margin_collapsing_quirk_foreign_namespace_parent_is_not_gated() {
        // Mirror of the previous test on the *parent* side: a
        // foreign-namespace element sharing the local name "body" must
        // not count as the real HTML `body` this quirk is scoped to,
        // even though the child otherwise satisfies every structural
        // condition rule 1 checks.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element_with_namespace(0, "body", "http://example.com/not-html", &[]);
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a foreign-namespace container sharing the local name \"body\" must \
             not count as the real body/td/th for the quirk"
        );
    }

    // ── float / clear wire-through (CSS2 §9.5, §9.5.2) ──

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
    fn deferred_projection_covers_supported_shorthands() {
        use crate::property::PropertyKey;

        fn parse_static(name: &str, source: &str) -> PropertyValue {
            let mut input = ParserInput::new(source);
            let mut parser = Parser::new(&mut input);
            let value = parse_value(name, &mut parser).expect("valid static shorthand");
            parser
                .expect_exhausted()
                .expect("static shorthand is exhaustive");
            value
        }

        let cases = vec![
            (
                "padding",
                "1px 2px 3px 4px",
                vec![
                    PropertyKey::PaddingTop,
                    PropertyKey::PaddingRight,
                    PropertyKey::PaddingBottom,
                    PropertyKey::PaddingLeft,
                ],
            ),
            (
                "margin-inline",
                "1px 2px",
                vec![PropertyKey::MarginLeft, PropertyKey::MarginRight],
            ),
            (
                "margin-block",
                "1px 2px",
                vec![PropertyKey::MarginTop, PropertyKey::MarginBottom],
            ),
            (
                "padding-inline",
                "1px 2px",
                vec![PropertyKey::PaddingLeft, PropertyKey::PaddingRight],
            ),
            (
                "padding-block",
                "1px 2px",
                vec![PropertyKey::PaddingTop, PropertyKey::PaddingBottom],
            ),
            (
                "margin",
                "1px 2px 3px 4px",
                vec![
                    PropertyKey::MarginTop,
                    PropertyKey::MarginRight,
                    PropertyKey::MarginBottom,
                    PropertyKey::MarginLeft,
                ],
            ),
            (
                "border",
                "1px solid red",
                vec![
                    PropertyKey::BorderTopWidth,
                    PropertyKey::BorderTopStyle,
                    PropertyKey::BorderTopColor,
                    PropertyKey::BorderRightWidth,
                    PropertyKey::BorderRightStyle,
                    PropertyKey::BorderRightColor,
                    PropertyKey::BorderBottomWidth,
                    PropertyKey::BorderBottomStyle,
                    PropertyKey::BorderBottomColor,
                    PropertyKey::BorderLeftWidth,
                    PropertyKey::BorderLeftStyle,
                    PropertyKey::BorderLeftColor,
                ],
            ),
            (
                "outline",
                "auto 2px red",
                vec![
                    PropertyKey::OutlineWidth,
                    PropertyKey::OutlineStyle,
                    PropertyKey::OutlineColor,
                ],
            ),
            (
                "overflow",
                "hidden scroll",
                vec![PropertyKey::OverflowX, PropertyKey::OverflowY],
            ),
            (
                "text-decoration",
                "underline wavy red",
                vec![
                    PropertyKey::TextDecorationLine,
                    PropertyKey::TextDecorationStyle,
                    PropertyKey::TextDecorationColor,
                ],
            ),
            (
                "flex",
                "2 3 10px",
                vec![
                    PropertyKey::FlexGrow,
                    PropertyKey::FlexShrink,
                    PropertyKey::FlexBasis,
                ],
            ),
            (
                "gap",
                "1px 2px",
                vec![PropertyKey::RowGap, PropertyKey::ColumnGap],
            ),
            (
                "place-content",
                "center space-between",
                vec![PropertyKey::AlignContent, PropertyKey::JustifyContent],
            ),
            (
                "grid-row",
                "2 / 5",
                vec![PropertyKey::GridRowStart, PropertyKey::GridRowEnd],
            ),
            (
                "grid-column",
                "main-start / main-end",
                vec![PropertyKey::GridColumnStart, PropertyKey::GridColumnEnd],
            ),
            (
                "place-items",
                "center stretch",
                vec![PropertyKey::AlignItems, PropertyKey::JustifyItems],
            ),
            (
                "place-self",
                "center stretch",
                vec![PropertyKey::AlignSelf, PropertyKey::JustifySelf],
            ),
        ];

        for (name, source, keys) in cases {
            let value = parse_static(name, source);
            for key in keys {
                assert!(project_deferred_value(value.clone(), key).is_some());
            }
            assert!(project_deferred_value(value, PropertyKey::Color).is_none());
        }
        assert!(project_deferred_value(PropertyValue::Color(RED), PropertyKey::Width).is_none());
    }

    #[test]
    fn direct_apply_ignores_precomputed_custom_values() {
        let initial = SpecifiedValues::initial();
        let mut specified = initial.clone();
        apply_value(
            PropertyValue::CustomProperty(CustomProperty {
                name: "--unused".into(),
                value: "red".into(),
            }),
            &mut specified,
        );
        apply_value(
            PropertyValue::Deferred(DeferredValue {
                property: "color".into(),
                value: "red".into(),
                key: PropertyKey::Color,
            }),
            &mut specified,
        );
        assert_eq!(specified, initial);
    }

    #[test]
    fn math_helpers_cover_nested_and_rejected_forms() {
        assert_eq!(
            simplify_math_functions(r#"rgb(calc(1px + 1px), 0, 0)"#),
            Some("rgb(2px, 0, 0)".into())
        );
        assert_eq!(
            simplify_math_functions(r#""calc(1px)" /* calc(2px) */"#),
            Some(r#""calc(1px)" /* calc(2px) */"#.into())
        );
        assert_eq!(
            simplify_math_functions("calc(calc(1px))"),
            Some("1px".into())
        );
        assert_eq!(
            simplify_math_functions_at_depth("calc(1px)", MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            None
        );
        assert_eq!(simplify_math_functions("(calc(1px))"), Some("(1px)".into()));
        assert_eq!(simplify_math_functions("[calc(1px)]"), Some("[1px]".into()));
        assert_eq!(simplify_math_functions("{calc(1px)}"), Some("{1px}".into()));
        assert!(needs_css_token_separator("-", "a"));
        assert!(needs_css_token_separator("+", "2"));
        assert!(needs_css_token_separator(".", "2"));
        assert!(needs_css_token_separator("#", "a"));
        assert!(needs_css_token_separator("10", "--foo"));
        assert!(needs_css_token_separator("10", "_foo"));
        assert!(needs_css_token_separator("10", r"\66 oo"));
        assert!(needs_css_token_separator("10", "é"));
        assert!(needs_css_token_separator("#", "1"));
        assert_eq!(
            evaluate_math_function("calc", &"x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1)),
            None
        );
        assert_eq!(
            evaluate_math_function("calc", "1 + 2"),
            Some("3".to_owned())
        );
        assert_eq!(evaluate_math_function("calc", "1 + 2px"), None);

        assert_eq!(evaluate_math_function("min", ""), None);
        assert_eq!(evaluate_math_function("min", "1px, 2em"), None);
        assert_eq!(evaluate_math_function("clamp", "1px, 2px"), None);
        assert_eq!(evaluate_math_function("clamp", "1px, 2em, 3px"), None);
        assert_eq!(evaluate_math_function("unknown", "1"), None);
        assert_eq!(
            serialize_math_value(MathValue {
                number: f32::INFINITY,
                unit: None,
            }),
            None
        );
        assert_eq!(
            serialize_math_value(MathValue {
                number: -0.0,
                unit: None,
            }),
            Some("0".to_owned())
        );
        let mut empty_values: Vec<MathValue> = Vec::new();
        assert_eq!(normalize_math_values(&mut empty_values), None);
        let mut overflowing_pair_left = MathValue {
            number: f32::MAX,
            unit: Some("in".to_owned()),
        };
        let mut overflowing_pair_right = MathValue {
            number: 1.0,
            unit: Some("px".to_owned()),
        };
        assert_eq!(
            normalize_math_pair(&mut overflowing_pair_left, &mut overflowing_pair_right),
            None
        );
        let mut overflowing_values = vec![
            MathValue {
                number: f32::MAX,
                unit: Some("in".to_owned()),
            },
            MathValue {
                number: 1.0,
                unit: Some("px".to_owned()),
            },
        ];
        assert_eq!(normalize_math_values(&mut overflowing_values), None);
        assert_eq!(
            serialize_math_value(MathValue {
                number: 1.0,
                unit: Some("x".repeat(MAX_SUBSTITUTED_VALUE_BYTES)),
            }),
            None
        );

        assert!(MathParser::new("1px 2px").parse().is_none());
        assert!(MathParser::new("1px+2px").parse().is_none());
        assert!(MathParser::new("1px +2px").parse().is_none());
        assert!(MathParser::new("1px + 2em").parse().is_none());
        assert_eq!(
            MathParser::new("-2px").parse().map(|value| value.number),
            Some(-2.0)
        );
        assert_eq!(
            MathParser::new("5px - 2px")
                .parse()
                .map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("3e38px + 3e38px").parse().is_none());
        assert!(MathParser::new("2px *").parse().is_none());
        assert_eq!(
            MathParser::new("2 * 3px").parse().map(|value| value.unit),
            Some(Some("px".to_owned()))
        );
        assert_eq!(
            MathParser::new("2px * 3").parse().map(|value| value.number),
            Some(6.0)
        );
        assert_eq!(
            MathParser::new("6px / 2").parse().map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("2px * 3px").parse().is_none());
        assert!(MathParser::new("2px / 0").parse().is_none());
        assert!(MathParser::new("3e38 * 3").parse().is_none());
        assert_eq!(
            MathParser::new("(1px + 2px)")
                .parse()
                .map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("(1px").parse().is_none());
        assert!(MathParser::new(".").parse().is_none());
        assert_eq!(
            MathParser::new("1.5px").parse().map(|value| value.number),
            Some(1.5)
        );
        assert_eq!(
            MathParser::new("1e-2px").parse().map(|value| value.number),
            Some(0.01)
        );
        assert!(MathParser::new("1e+px").parse().is_none());
        assert_eq!(
            MathParser::new("50%").parse().map(|value| value.unit),
            Some(Some("%".to_owned()))
        );

        assert_eq!(
            split_top_level_commas(r#""a,b", 1px"#),
            Some(vec![r#""a,b""#, "1px"])
        );
        assert_eq!(
            split_top_level_commas("1px /*,*/, 2px"),
            Some(vec!["1px /*,*/", "2px"])
        );
        assert_eq!(
            split_top_level_commas("min(1px, 2px), 3px"),
            Some(vec!["min(1px, 2px)", "3px"])
        );
        assert_eq!(
            split_top_level_commas(r"foo\,bar, baz"),
            Some(vec![r"foo\,bar", "baz"])
        );
        assert_eq!(split_top_level_commas("{a,b}, c"), Some(vec!["{a,b}", "c"]));
        assert_eq!(split_top_level_commas(")"), None);
        assert_eq!(split_top_level_commas("[a,b"), None);
        let nested = format!(
            "{}1{}",
            "(".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            ")".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert!(MathParser::new(&nested).parse().is_none());
        let too_deep = format!(
            "{}1{}",
            "[".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            "]".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert_eq!(split_top_level_commas(&too_deep), None);
    }

    #[test]
    fn variable_scanner_handles_literals_bounds_and_fallbacks() {
        let mut local = HashMap::from([(SmolStr::from("--a"), SmolStr::from("red"))]);
        let inherited = CustomPropertyEnvironment::from_map(HashMap::new());
        {
            let mut resolver = CustomPropertyResolver {
                local: &local,
                inherited: &inherited,
                cycle_members: HashSet::new(),
                memo: HashMap::new(),
                resolving: Vec::new(),
                budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
            };
            assert_eq!(resolver.resolve("--a", 0), Some("red".into()));
            assert_eq!(resolver.resolve("--a", 0), Some("red".into()));
        }
        let mut resolver = CustomPropertyResolver {
            local: &local,
            inherited: &inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: vec!["--a".into()],
            budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
        };
        assert_eq!(resolver.resolve("--a", 0), None);
        let branching_local = HashMap::from([
            (
                SmolStr::from("--root"),
                SmolStr::from("var(--shared) var(--shared)"),
            ),
            (SmolStr::from("--shared"), SmolStr::from("red")),
        ]);
        let branching_inherited = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut resolver = CustomPropertyResolver {
            local: &branching_local,
            inherited: &branching_inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: Vec::new(),
            budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
        };
        assert_eq!(resolver.resolve("--root", 0), Some("red red".into()));
        assert!(resolver.memo.contains_key(&(SmolStr::from("--shared"), 1)));

        let budget_local = HashMap::from([
            (SmolStr::from("--a"), SmolStr::from("var(--b)")),
            (SmolStr::from("--b"), SmolStr::from("var(--c)")),
            (SmolStr::from("--c"), SmolStr::from("red")),
        ]);
        let mut resolver = CustomPropertyResolver {
            local: &budget_local,
            inherited: &inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: Vec::new(),
            budget: VariableResolutionBudget::new(2),
        };
        assert_eq!(resolver.resolve("--a", 0), None);
        assert!(resolver.budget.exhausted);
        local.insert("--broken".into(), "\"unterminated".into());
        assert!(find_cycle_members(&local).is_empty());

        let mut references = Vec::new();
        assert!(
            collect_var_references(
                r#""var(--ignored)" /* var(--also-ignored) */ var(--used)"#,
                &mut references
            )
            .is_ok()
        );
        assert_eq!(references, vec![SmolStr::from("--used")]);
        assert!(collect_var_references("\"unterminated", &mut Vec::new()).is_err());
        assert!(collect_var_references("/* unterminated", &mut Vec::new()).is_err());

        fn no_resolution(_: &str) -> Option<SmolStr> {
            None
        }
        assert_eq!(
            substitute_vars(
                r#""var(--ignored)" /* var(--also-ignored) */ blue"#,
                &mut no_resolution,
                0
            ),
            Some(r#""var(--ignored)" /* var(--also-ignored) */ blue"#.into())
        );
        assert_eq!(
            substitute_vars("var(--missing, blue)", &mut no_resolution, 0),
            Some("blue".into())
        );
        assert_eq!(
            substitute_vars(
                "var(--missing, var(--also-missing, red))",
                &mut no_resolution,
                0
            ),
            Some("red".into())
        );
        assert_eq!(
            substitute_vars(
                "var(--x)",
                &mut |_| Some("x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1).into()),
                0
            ),
            None
        );
        assert_eq!(
            substitute_vars("var(--x)", &mut |_| None, MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            None
        );

        assert_eq!(skip_css_string(r#""a\"b""#, 0), Some(6));
        assert_eq!(skip_css_string("\"unterminated", 0), None);
        assert_eq!(skip_css_comment("/* comment */", 0), Some(13));
        assert_eq!(skip_css_comment("/* unterminated", 0), None);
        assert_eq!(skip_css_escape("x", 0), None);
        assert_eq!(skip_css_escape("\\\n", 0), None);
        assert_eq!(skip_css_escape(r"\31 ", 0), Some(4));
        assert_eq!(skip_css_escape("\\31\r\n", 0), Some(5));
        assert_eq!(
            split_var_arguments("--x, var(--y, red)"),
            Some((SmolStr::from("--x"), Some("var(--y, red)")))
        );
        assert_eq!(
            split_var_arguments("--x, \"a,b\" /* comment */"),
            Some((SmolStr::from("--x"), Some("\"a,b\" /* comment */")))
        );
        assert_eq!(
            split_var_arguments("--x, foo(bar)"),
            Some((SmolStr::from("--x"), Some("foo(bar)")))
        );
        assert_eq!(split_var_arguments("color, red"), None);
        assert_eq!(split_var_arguments("\"--x\", red"), None);
        assert_eq!(
            split_var_arguments("--x /* comment */, red"),
            Some((SmolStr::from("--x"), Some("red")))
        );
        assert_eq!(split_var_arguments("--x(foo), red"), None);
        let (escaped_name, escaped_fallback) = split_var_arguments("--x\\,, red").unwrap();
        assert_eq!(escaped_name, "--x,");
        assert_eq!(escaped_fallback, Some("red"));
        assert_eq!(split_var_arguments("--x)"), None);
    }

    #[test]
    fn variable_substitution_preserves_number_identifier_boundary() {
        assert_eq!(
            substitute_vars("var(--n)--foo", &mut |_| Some("10".into()), 0),
            Some("10 --foo".into())
        );
        assert_eq!(
            substitute_vars("var(--missing, [foo)bar])", &mut |_| None, 0),
            Some("[foo)bar]".into())
        );
    }

    #[test]
    fn component_value_scanners_bound_nesting_and_blocks() {
        let nested = format!(
            "{}var(--x){}",
            "[".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            "]".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert_eq!(find_function_tokens(&nested, &["var"]), None);
        assert_eq!(split_top_level_commas("[a,b], c"), Some(vec!["[a,b]", "c"]));
    }

    #[test]
    fn custom_property_substitutes_into_inherited_color() {
        let cv = cascade_doc("", "p", Some("--accent: red; color: var(--accent)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn var_substitutes_inside_a_nested_function() {
        let cv = cascade_doc("", "p", Some("--red: 255; color: rgb(var(--red), 0, 0)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn custom_property_inherits_to_child() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("--accent: blue"));
        let child = doc.push_element(parent, "p", Some("color: var(--accent, red)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, BLUE);
    }

    /// A deep chain keeps one local custom-property delta per element instead
    /// of cloning the complete inherited map into each computed value.
    #[test]
    fn deep_custom_property_chain_keeps_persistent_environment_deltas() {
        use std::fmt::Write as _;

        const DEPTH: usize = 256;
        let mut doc = TestDoc::new();
        let mut parent = 0;
        let mut ids = Vec::with_capacity(DEPTH);
        for index in 0..DEPTH {
            let mut style = String::new();
            write!(style, "--chain-{index}: {index}px").unwrap();
            let id = doc.push_element(parent, "div", Some(&style));
            ids.push(id);
            parent = id;
        }

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");

        // Every environment owns exactly this element's declaration. The
        // complete chain is shared through parent Arc pointers, so retained
        // local entries are O(N), not O(N²).
        let mut total_local_entries = 0;
        for (index, id) in ids.iter().copied().enumerate() {
            let environment = &result.computed[id].custom_properties;
            assert_eq!(environment.local_entry_count(), 1);
            total_local_entries += environment.local_entry_count();
            if index > 0 {
                let parent_environment = environment
                    .parent_environment()
                    .expect("non-root custom environment has a parent");
                // cov:ignore: panic-message literal only executed on assertion
                // failure, which does not happen while this test passes.
                assert!(
                    std::sync::Arc::ptr_eq(
                        parent_environment,
                        &result.computed[ids[index - 1]].custom_properties
                    ),
                    "custom environment must share the immediate ancestor's environment"
                );
            }
        }
        assert_eq!(total_local_entries, DEPTH);
        let leaf_environment = &result.computed[*ids.last().unwrap()].custom_properties;
        assert_eq!(leaf_environment.get("--chain-0"), Some("0px".into()));
        assert_eq!(leaf_environment.get("--chain-255"), Some("255px".into()));
    }

    #[test]
    fn invalid_var_keeps_inherited_property_value() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("color: red"));
        let child = doc.push_element(parent, "p", Some("color: var(--missing)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, RED);
    }

    #[test]
    fn invalid_var_uses_initial_for_non_inherited_property() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("width: 20px"));
        let child = doc.push_element(parent, "p", Some("width: var(--missing)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.computed[child].width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn invalid_custom_property_overrides_inherited_value() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("--accent: red"));
        let child = doc.push_element(
            parent,
            "p",
            Some("--accent: var(--missing); color: var(--accent, blue)"),
        );
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, BLUE);
    }

    #[test]
    fn missing_custom_property_uses_var_fallback() {
        let cv = cascade_doc("", "p", Some("color: var(--missing, blue)"));
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn custom_property_rejects_top_level_bang_except_important() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: !not-important; color: var(--accent, blue)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn custom_property_important_wins_and_is_stripped_before_substitution() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: red !important; --accent: blue; color: var(--accent)"),
        );
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn custom_property_names_are_case_sensitive() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: red; --Accent: blue; color: var(--Accent)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn cyclic_custom_property_uses_var_fallback() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--a: var(--b); --b: var(--a); color: var(--a, blue)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn fallback_references_do_not_rescue_a_custom_property_cycle() {
        let cv = cascade_doc(
            "",
            "p",
            Some(
                "--a: var(--b, red); --b: var(--a, blue); \
                 color: var(--a)",
            ),
        );
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn overly_deep_finite_variable_chain_is_bounded() {
        use std::fmt::Write as _;

        let mut css = String::new();
        for index in 0..256 {
            write!(css, "--v{index}: var(--v{}); ", index + 1).unwrap();
        }
        css.push_str("--v256: red; color: var(--v0)");

        let cv = cascade_doc("", "p", Some(&css));
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn overly_deep_nested_var_fallback_is_bounded() {
        let mut fallback = String::from("red");
        for index in 0..256 {
            fallback = format!("var(--missing-{index}, {fallback})");
        }
        let css = format!("color: {fallback}");

        let cv = cascade_doc("", "p", Some(&css));
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn calc_substituted_custom_property_reaches_width() {
        let cv = cascade_doc(
            "",
            "div",
            Some("--spacing: 10px; width: calc(var(--spacing) + 5px)"),
        );
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(15.0));
    }

    #[test]
    fn invalid_math_declaration_is_dropped_before_cascade() {
        let cv = cascade_doc("", "div", Some("width: 10px; width: calc(foo)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    #[test]
    fn hash_prefixed_var_text_is_not_substituted() {
        let cv = cascade_doc("", "div", Some("color: red; color: #var(--missing)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn min_max_and_clamp_reach_width() {
        let min = cascade_doc("", "div", Some("width: min(20px, 10px)"));
        let max = cascade_doc("", "div", Some("width: max(10px, 20px)"));
        let clamp = cascade_doc("", "div", Some("width: clamp(5px, 20px, 10px)"));
        assert_eq!(min.width, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(max.width, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(clamp.width, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    #[test]
    fn var_substitution_does_not_join_adjacent_tokens() {
        // `var(--n)px` is not a valid way to form a dimension: substitution
        // happens at token level, so the result is the two-token sequence
        // `10` + `px`, not a newly reparsed `10px` token.
        let cv = cascade_doc("", "div", Some("--n: 10; width: var(--n)px"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(simplify_math_functions("calc(10)px"), Some("10 px".into()));
    }

    #[test]
    fn var_substitution_does_not_create_a_function_token() {
        let cv = cascade_doc("", "div", Some("--fn: rgb; color: var(--fn)(255, 0, 0)"));
        assert_eq!(cv.color, CssColor::BLACK);
        assert_eq!(
            substitute_vars("var(--fn)(255, 0, 0)", &mut |_| Some("rgb".into()), 0),
            Some("rgb (255, 0, 0)".into())
        );
    }

    #[test]
    fn escaped_math_function_names_are_evaluated_after_token_decoding() {
        assert_eq!(simplify_math_functions(r"c\61 lc(1px)"), Some("1px".into()));
        let cv = cascade_doc("", "div", Some(r"width: c\61 lc(1px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(1.0));
    }

    #[test]
    fn css_comments_are_whitespace_inside_math_functions() {
        assert_eq!(
            simplify_math_functions("calc(1px /* comment */ + 2px)"),
            Some("3px".into())
        );
        let cv = cascade_doc("", "div", Some("width: calc(1px /* comment */ + 2px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(3.0));
    }

    #[test]
    fn variable_resolution_budget_rejects_excess_expansions() {
        let mut budget = VariableResolutionBudget::new(2);
        assert!(budget.consume());
        assert!(budget.consume());
        assert!(!budget.consume());

        let mut budget = VariableResolutionBudget::new(0);
        assert_eq!(
            substitute_vars_with_budget("var(--x)", &mut |_| Some("red".into()), 0, &mut budget),
            None
        );
    }

    #[test]
    fn has_css_whitespace_before_at_start_returns_false() {
        assert!(!MathParser::new("1px").has_css_whitespace_before(0));
    }

    #[test]
    fn compatible_absolute_length_units_are_normalized_in_math() {
        let cv = cascade_doc("", "div", Some("width: calc(1in + 96px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(192.0));
        assert_eq!(
            evaluate_math_function("calc", "1in + 96px"),
            Some("192px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("min", "1in, 96px"),
            Some("96px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("max", "1in, 96px"),
            Some("96px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("clamp", "1in, 96px, 2in"),
            Some("96px".to_owned())
        );
    }

    #[test]
    fn mixed_length_percentage_math_is_intentionally_not_supported() {
        // The current public computed representation keeps `Length::Percent`
        // separate from absolute lengths and has no containing-block basis at
        // computed-value substitution time. Keep this bounded design explicit
        // until the representation/API grows a typed length-percentage sum.
        let cv = cascade_doc("", "div", Some("width: calc(10px + 5%)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn oversized_variable_and_math_inputs_are_rejected() {
        let oversized = "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1);
        assert!(collect_var_references(&oversized, &mut Vec::new()).is_err());
        assert_eq!(split_top_level_commas(&oversized), None);
        assert_eq!(split_var_arguments(&oversized), None);
    }

    #[test]
    fn clamp_min_wins_when_bounds_are_reversed() {
        let cv = cascade_doc("", "div", Some("width: clamp(20px, 0px, 10px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(20.0));
    }
}
