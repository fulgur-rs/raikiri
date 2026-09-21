use std::collections::HashMap;
use std::ops::Range;

use cssparser::{Parser, ParserInput};
use selectors::parser::Selector;

use crate::PseudoElem;
use crate::RaikiriSelectorImpl;
use crate::media::MediaContext;
use crate::property::{CustomProperty, PropertyValue};
use crate::rule::{expand_shorthand_into, parse_declaration_block};
use crate::ruletree::{Origin, RuleTree};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

use super::html_quirks::{push_img_dimension_hints, push_margin_collapsing_quirk_declarations};
use super::selector_match::{match_complex_selector_list, selector_matches_pseudo_element};

/// selectors 由来の 32-bit specificity。u32 で完全順序比較。
pub(crate) type Specificity = u32;

/// inline style の specificity。CSS Cascading L4 §6.1 "Cascade Sorting Order"
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の Specificity 段 verbatim:
/// "declarations that do not belong to a style rule (such as the contents of a
/// style attribute) are considered to have a specificity higher than any
/// selector." (`(1, 0, 0, 0)` は CSS 2.1 §6.4.3 の旧表現であり L4 の規定ではない)。
/// selectors crate は 32-bit packed で `id << 20 | class << 10 | element` を使う。
/// `1 << 30` はその packed 空間のどの selector 由来 specificity よりも大きいので、
/// 上記 "higher than any selector" を満たす。**この margin はちょうど 1** であり
/// upstream が packing 幅を広げると反転しうる不変条件 — check は
/// `tests::inline_specificity_exceeds_max_reachable_packed_specificity` を参照。
pub(crate) const INLINE_SPECIFICITY: Specificity = 1 << 30;
/// inline style の source_order — 全 stylesheet rule より後 (最終出現扱い)。
pub(crate) const INLINE_SOURCE_ORDER: u32 = u32::MAX;

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
pub(crate) const PRESENTATIONAL_HINT_SPECIFICITY: Specificity = 0;
/// 同 hint の source_order。`0` — 実 stylesheet の最初の rule
/// ([`crate::ruletree::RuleTree::add_stylesheet`] は空 `RuleTree` への
/// 最初の rule に `source_order = 0` を採番する) と数値上 tie し得る値だが、
/// hint 専用の `cascade_rank` tier ([`Origin::AuthorPresentationalHint`]) を
/// 導入して以降、この tie は実際には発生しない — rank 差が specificity/source_order より先に
/// tuple compare で決着するため ([`push_img_dimension_hints`] doc の
/// "Cascade origin" 節参照)。`0` という値自体は「他候補と衝突しない値」を
/// 意図したものではなく、単に real stylesheet rule の source_order と同じ
/// 値域を使うという単純さのための選択。
pub(crate) const PRESENTATIONAL_HINT_SOURCE_ORDER: u32 = 0;

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
pub(crate) const MARGIN_COLLAPSING_QUIRK_SPECIFICITY: Specificity = INLINE_SPECIFICITY;
/// [`MARGIN_COLLAPSING_QUIRK_SPECIFICITY`]'s companion source_order. `0` —
/// same reasoning as [`PRESENTATIONAL_HINT_SOURCE_ORDER`]: the
/// dedicated `cascade_rank` tier already decides every comparison that
/// matters (against other `Origin::UserAgent` declarations, `beats`'s
/// specificity comparison is what actually separates this from
/// minimal.css's rule, and no two of *this* function's own pushes ever
/// compete against each other for the same property on the same node), so
/// no value other than "the same low end of the range real stylesheet
/// source_order uses" is needed here.
pub(crate) const MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER: u32 = 0;

/// 1 candidate declaration = `(value, important, origin, specificity, source_order)`。
/// `collect_cascaded` が populate、`pick_winners` が rank 化して winner を選ぶ
/// (`Origin` を含む — clippy::type_complexity 回避のため alias 化)。
pub(crate) type CascadedDecl = (PropertyValue, bool, Origin, Specificity, u32);

/// A custom-property candidate. Unlike ordinary declarations, custom
/// properties are keyed by their case-sensitive name rather than by a fixed
/// `PropertyKey` slot.
pub(crate) type CustomCascadedDecl = (CustomProperty, bool, Origin, Specificity, u32);

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
/// 別 node の候補を静かに拾う。通常の呼び出し側 (実装コード) には
/// [`candidates`](Self::candidates) だけを使わせることで、
/// 「この node 自身の区間ちょうど」以外のスライスを組み立てさせない。
/// `decls`/`ranges` フィールド自体は `pub(crate)` — no-overlap 不変条件
/// (どの 2 node の区間も重ならない) を直接検証するテストのための例外的な
/// 白箱アクセス経路であり、実装コードはこの 2 フィールドを直接読まない
/// (常に `candidates`/`custom_candidates`/`pseudo_candidates`/
/// `pseudo_custom_candidates` 経由)。
///
/// `pub(crate)` は [`resolve_inheritance`] 自身が `pub(crate)` (他 module の
/// doc からの intra-doc link のため) であることに追随するだけで、他 module
/// から構築/操作されることは想定していない — 構築は [`cascade`] が行い、
/// 内容の書き込みは [`collect_cascaded`] に閉じている (いずれも本 module)。
pub(crate) struct CascadedArena {
    /// 全 node の candidate を document 内 visit 順で連結した flat 領域。
    pub(crate) decls: Vec<CascadedDecl>,
    /// node ごとの `decls` 内部分区間。空 (no candidate) の node はここに
    /// entry を持たない — 旧実装の `if !per_node.is_empty() { out.insert(..) }`
    /// と同じ「無ければ握らない」契約。
    pub(crate) ranges: HashMap<StyleNodeId, Range<usize>>,
    /// All custom-property candidates in document visit order.
    custom_decls: Vec<CustomCascadedDecl>,
    /// Per-node ranges into `custom_decls`.
    custom_ranges: HashMap<StyleNodeId, Range<usize>>,
    /// `decls`/`ranges` と同じ flat-arena 技法だが、key が「originating
    /// element の `StyleNodeId`」ではなく「`(originating element の
    /// StyleNodeId, その `::before`/`::after`)`」— 1 element が `::before`/
    /// `::after` 両方の候補を独立に持ちうるため。空 (no candidate) の
    /// `(id, pseudo)` の組はここに entry を持たない (同じ「無ければ握らない」
    /// 契約)。
    pseudo_decls: Vec<CascadedDecl>,
    /// `(id, pseudo)` ごとの `pseudo_decls` 内部分区間。
    pseudo_ranges: HashMap<(StyleNodeId, PseudoElem), Range<usize>>,
    /// `pseudo_decls` の custom-property 版。
    pseudo_custom_decls: Vec<CustomCascadedDecl>,
    /// `(id, pseudo)` ごとの `pseudo_custom_decls` 内部分区間。
    pseudo_custom_ranges: HashMap<(StyleNodeId, PseudoElem), Range<usize>>,
}

impl CascadedArena {
    pub(crate) fn new() -> Self {
        Self {
            decls: Vec::new(),
            ranges: HashMap::new(),
            custom_decls: Vec::new(),
            custom_ranges: HashMap::new(),
            pseudo_decls: Vec::new(),
            pseudo_ranges: HashMap::new(),
            pseudo_custom_decls: Vec::new(),
            pseudo_custom_ranges: HashMap::new(),
        }
    }

    /// `id` 自身の candidate 一覧 — [`pick_winners`] にそのまま渡せる
    /// **自 node 専用**のスライス。
    ///
    /// 返す slice は常に `&self.decls[range]` で `range` は `id` のために
    /// `collect_cascaded` が積んだ区間ちょうど — 呼び出し側がこれ以外の形の
    /// slice (全体、あるいは `range.start` だけずらしたもの) を組み立てる経路は
    /// 本 struct に存在しない。
    pub(crate) fn candidates(&self, id: StyleNodeId) -> Option<&[CascadedDecl]> {
        self.ranges.get(&id).map(|range| &self.decls[range.clone()])
    }

    pub(crate) fn custom_candidates(&self, id: StyleNodeId) -> Option<&[CustomCascadedDecl]> {
        self.custom_ranges
            .get(&id)
            .map(|range| &self.custom_decls[range.clone()])
    }

    /// `(id, pseudo)` の candidate 一覧 — [`candidates`](Self::candidates) の
    /// `::before`/`::after` 版。
    pub(crate) fn pseudo_candidates(
        &self,
        id: StyleNodeId,
        pseudo: PseudoElem,
    ) -> Option<&[CascadedDecl]> {
        self.pseudo_ranges
            .get(&(id, pseudo))
            .map(|range| &self.pseudo_decls[range.clone()])
    }

    pub(crate) fn pseudo_custom_candidates(
        &self,
        id: StyleNodeId,
        pseudo: PseudoElem,
    ) -> Option<&[CustomCascadedDecl]> {
        self.pseudo_custom_ranges
            .get(&(id, pseudo))
            .map(|range| &self.pseudo_custom_decls[range.clone()])
    }
}

/// Pushes one declaration into `decls`/`custom_decls` (routing on
/// `PropertyValue::CustomProperty`, same split every candidate list in this
/// module uses). Takes the destination `Vec`s directly rather than a whole
/// [`CascadedArena`] so the same function serves both the real-element path
/// (`&mut out.decls, &mut out.custom_decls`, [`collect_cascaded`]) and the
/// `::before`/`::after` path (a per-element scratch buffer pair,
/// [`collect_cascaded`]'s pseudo-element section) without duplicating this
/// match.
fn push_cascaded_decl(
    decls: &mut Vec<CascadedDecl>,
    custom_decls: &mut Vec<CustomCascadedDecl>,
    value: PropertyValue,
    important: bool,
    origin: Origin,
    specificity: Specificity,
    source_order: u32,
) {
    match value {
        PropertyValue::CustomProperty(custom) => {
            custom_decls.push((custom, important, origin, specificity, source_order))
        }
        value => decls.push((value, important, origin, specificity, source_order)),
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
pub(crate) struct RankedDecl {
    /// [`cascade_rank`] の origin + `!important` 優先度。
    pub(crate) rank: u8,
    /// selector specificity (inline style は [`INLINE_SPECIFICITY`])。
    pub(crate) specificity: Specificity,
    /// stylesheet 内出現順 (inline style は [`INLINE_SOURCE_ORDER`])。
    pub(crate) source_order: u32,
    /// [`pick_winners`] に渡された `candidates` slice 内の位置。
    pub(crate) idx: usize,
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
/// 定める (transition/animation の 2 項目は本 crate 未実装のため以下では省略):
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
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>) が定める "author presentational hint origin"。L5 自身の §6.1 も
/// 上記と同じ 8 項目リストのままで hint origin をそこに明示的には
/// 挿入しない (hint の位置付けは §6.5 側の
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
// Keep the default-context helper as the stable internal entry point named by
// the surrounding cascade documentation; production dispatch uses the
// context-aware implementation below.
#[allow(dead_code)]
pub(crate) fn collect_cascaded<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut CascadedArena,
) {
    collect_cascaded_with_media_context(dom, id, rule_tree, out, &MediaContext::default());
}

pub(crate) fn collect_cascaded_with_media_context<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut CascadedArena,
    media_context: &MediaContext,
) {
    // Document-wide constant — read once rather than
    // per (node, rule) pair inside the loop below.
    let quirks_mode = dom.quirks_mode();
    let mut style_rules = rule_tree
        .style_rules
        .iter()
        .map(|rule| (rule, None))
        .collect::<Vec<_>>();
    style_rules.extend(
        rule_tree
            .media_rules
            .iter()
            .map(|media| (&media.rule, Some(media.condition))),
    );
    style_rules.sort_unstable_by_key(|(rule, _)| rule.source_order);

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
    // `::before`/`::after` candidate scratch buffers — declared outside the
    // walk loop and drained (via `Vec::append`, see the flush site below) at
    // the end of each element's processing, so they're always empty when a
    // new element starts. Reused across the whole document walk rather than
    // allocated fresh per element, same rationale as `resolve_inheritance`'s
    // `winners` buffer.
    let mut pseudo_before_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_after_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_marker_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_before_custom: Vec<CustomCascadedDecl> = Vec::new();
    let mut pseudo_after_custom: Vec<CustomCascadedDecl> = Vec::new();
    let mut pseudo_marker_custom: Vec<CustomCascadedDecl> = Vec::new();
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
                // continues to check the outcome this comment claims.
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
                for (rule, media_condition) in &style_rules {
                    if media_condition
                        .as_ref()
                        .is_some_and(|condition| !condition.matches(media_context))
                    {
                        continue;
                    }
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
                                    &mut out.decls,
                                    &mut out.custom_decls,
                                    d.value,
                                    d.important,
                                    rule.origin,
                                    spec,
                                    rule.source_order,
                                );
                            });
                        }
                    }
                    // `::before`/`::after` — independent pass over the same
                    // rule's selector list (a rule's comma-separated list can
                    // target the real element via one selector and a
                    // pseudo-element via another, e.g. `a, a::before {..}`,
                    // so this isn't mutually exclusive with the match above).
                    // `selector_matches_pseudo_element` fast-returns `None`
                    // via `Selector::pseudo_element()`'s `O(1)` flag check
                    // for the (overwhelmingly common) selector that doesn't
                    // target a pseudo-element at all, so this second list
                    // walk stays cheap for documents with no `::before`/
                    // `::after` rules.
                    for selector in rule.selectors.slice() {
                        let Some(pseudo) = selector_matches_pseudo_element(
                            dom,
                            selector,
                            &elem,
                            id,
                            &ancestor_path,
                            quirks_mode,
                        ) else {
                            continue;
                        };
                        let spec = specificity_of(selector);
                        let (buf, custom_buf) = match pseudo {
                            PseudoElem::Before => {
                                (&mut pseudo_before_decls, &mut pseudo_before_custom)
                            }
                            PseudoElem::After => {
                                (&mut pseudo_after_decls, &mut pseudo_after_custom)
                            }
                            PseudoElem::Marker => {
                                (&mut pseudo_marker_decls, &mut pseudo_marker_custom)
                            }
                        };
                        for decl in &rule.declarations {
                            expand_shorthand_into(decl, |d| {
                                push_cascaded_decl(
                                    buf,
                                    custom_buf,
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
                            &mut out.decls,
                            &mut out.custom_decls,
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
                // Flush this element's `::before`/`::after` scratch buffers
                // into the shared pseudo arena — `Vec::append` moves (no
                // clone) and leaves the scratch buffer empty, ready for the
                // next element that has a pseudo match to reuse without a
                // fresh allocation (same "declare outside the loop, drain in
                // place" idiom `resolve_inheritance`'s `winners` buffer
                // uses). Only elements with an actual match ever touch these
                // buffers, so they stay empty (a cheap `is_empty` `Vec`, no
                // allocation) for the common no-`::before`/`::after` case.
                for (pseudo, buf, custom_buf) in [
                    (
                        PseudoElem::Before,
                        &mut pseudo_before_decls,
                        &mut pseudo_before_custom,
                    ),
                    (
                        PseudoElem::After,
                        &mut pseudo_after_decls,
                        &mut pseudo_after_custom,
                    ),
                    (
                        PseudoElem::Marker,
                        &mut pseudo_marker_decls,
                        &mut pseudo_marker_custom,
                    ),
                ] {
                    let pseudo_start = out.pseudo_decls.len();
                    out.pseudo_decls.append(buf);
                    if out.pseudo_decls.len() > pseudo_start {
                        out.pseudo_ranges
                            .insert((id, pseudo), pseudo_start..out.pseudo_decls.len());
                    }
                    let pseudo_custom_start = out.pseudo_custom_decls.len();
                    out.pseudo_custom_decls.append(custom_buf);
                    if out.pseudo_custom_decls.len() > pseudo_custom_start {
                        out.pseudo_custom_ranges.insert(
                            (id, pseudo),
                            pseudo_custom_start..out.pseudo_custom_decls.len(),
                        );
                    }
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
        } // cov:ignore: fallthrough-vs-continue region split inside a loop body; every test with an in-document element already exercises this closing brace (verified against a main-branch baseline, see raikiri-spike-4nhl.9).
    }
}
pub(crate) fn specificity_of(selector: &Selector<RaikiriSelectorImpl>) -> Specificity {
    // selectors crate の Selector::specificity は 32-bit packed integer を返す。
    selector.specificity()
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
pub(crate) fn pick_winners(candidates: &[CascadedDecl], winners: &mut Vec<Option<RankedDecl>>) {
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

pub(crate) fn beats(candidate: RankedDecl, existing: RankedDecl) -> bool {
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
