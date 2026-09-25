use super::*;

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
/// plain な [`DisplayValue::Inline`] の node は、direct text children
/// だけなら本条件を満たさない: `block` / `inline-block` と異なり、
/// non-replaced な `inline` box は CSS 2.1 §9.2.1.1 上、自身の content に
/// 対して block container を生成しない — その content は `inline` box 自身と
/// *同じ* inline formatting context に流れ込む ordinary な inline-level
/// content である。nested な inline element child を持つ場合だけは例外として
/// synthetic flex line root を作る。taffy の block path ではその child の
/// descendants が縦に積まれるためであり、これは DOM の flatten ではなく
/// nested wrapper ごとの最小 line-box bridge である。
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
/// `flex-direction` / `flex-wrap` 宣言を `bridge_flex` が既に copy
/// 済みの可能性があり、ここで flex container になった時点でそれが
/// inert でなくなるため — default のままだろうと期待するのではなく
/// 明示的に上書きする必要がある。
///
/// `align_items: Baseline` を set するのは、`bridge_alignment` が copy
/// した author の `align-items` を上書きし、参加する text children の
/// first baseline を揃えるためである。`taffy_impl.rs` は text leaf の
/// Parley first-line baseline を report し、`inline` / `inline-block`
/// wrapper では in-flow text baseline を box へ伝える。baseline を持たない
/// replaced box 等は taffy の bottom-edge fallback に残る。`vertical-align`
/// の top / bottom は child の `align_self` で引き続き別扱いする。
///
/// `vertical-align` の実装済み length / sub / super shift は paint 時に
/// glyph へ適用する一方、その shift 分を line root の synthetic leading /
/// trailing として sizing に加える。これにより raised / lowered box が
/// line box の高さへ参加し、他の child も同じ text baseline を基準に配置
/// される。Synthetic extent は authored padding に加算されるため、既存の
/// padding 値と共存する。
///
/// container 自身は `justify_content: None` (taffy 自身の default、CSS
/// の `normal` 相当) へも reset する — 全く同じ「もう inert ではない」
/// 理由による: `bridge_alignment` が copy した author の
/// `justify-content` 宣言は、block container だった間は inert だったが、
/// ここで flex container になった時点で main-axis 方向の item 配置
/// (space-between 等) を変えてしまう。CSS 2.1 の inline formatting
/// context に "line box 上の複数 box を main-axis 方向に再配置する"
/// 意味論はそもそも存在しないため、taffy 側の default に戻すことでその
/// 意味論を無効化する。`gap` (`row-gap` / `column-gap`) も同じ理由で
/// `0` へ reset する — line box は inline-level box の間に author 指定の
/// 隙間を空ける意味論を持たない (CSS 2.1 §9.4.2 の line box は隙間なく
/// box を並べるモデル) ため、`bridge_gap` が copy した author 値を
/// ここで無効化する。
///
/// `align_content` も `bridge_alignment` が copy した author の値を
/// `Some(FlexStart)` へ reset する。これは複数 flex line 間で余った
/// cross-axis space を配る flex 専用 property であり、CSS inline context
/// には対応する意味論がない。reset を怠ると明示的な `height` がある場合に
/// default の `Stretch` が余り space を `<br>` の zero-height line に配り、
/// 視覚的な gap を作る。
///
/// 参加する各 child はさらに `flex_grow: 0.0` / `flex_shrink: 0.0` /
/// `flex_basis: auto` / `align_self: None` (or `Top` / `Bottom` override) も得る (`bridge_flex` /
/// `bridge_alignment` が自身の author CSS から copy した値を、上記と
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
/// 戻す理由は、container 側の `align_items: Baseline` へ一貫して
/// fallback させるためである (`auto` は親の `align-items` へ fallback
/// する契約、`bridge_alignment` の doc 参照)。`vertical-align: top` /
/// `bottom` はそれぞれ `FlexStart` / `FlexEnd` に明示変換する。
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
/// `<br>` を含む container にのみ生じ、これまで check されていた
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
/// - nested な inline element を ancestor の DOM line box へ flatten する
///   ことは行わない。nested wrapper が inline element child を持つ場合は
///   wrapper自身を最小 flex line root にするが、各 wrapper の children は
///   その wrapper 内で独立に layout する。
/// - [`preshape_text`] は本 pass の後 (`apply_computed_to_style` 完了後、
///   `layout_single_page` の後段) に、各 text node の glyph run を full
///   page width に対して shape・soft-wrap する。これは本 pass が最終的に
///   その text node に line box 上の sibling と並べて与える横幅とは
///   無関係であり、text の shaping を実際の available な inline space
///   と整合させることは follow-up work であり、ここでは行わない。
/// - `text-align: center` は本 pass と [`realign_text_after_layout`] の
///   2 経路で扱う: qualify した container 自身は line box 全体を中央に寄せる
///   ため `justify_content` を `Center` にする (他値は `None` のまま —
///   `Right` / `Justify` 等の flex 対応は本 task の scope 外)。
///   qualify しない block container 内の単独 Text は後段の
///   `realign_text_after_layout` が parley 側で中央寄せする。
///   詳細は同関数の doc 参照。
fn text_align_to_parley(v: TextAlign) -> Alignment {
    match v {
        TextAlign::Start => Alignment::Start,
        TextAlign::End => Alignment::End,
        TextAlign::Left => Alignment::Left,
        TextAlign::Right => Alignment::Right,
        TextAlign::Center => Alignment::Center,
        TextAlign::Justify => Alignment::Justify,
        // CSS Text 3 §6.1: `justify-all` は全行 (最終行含む) を justify。
        // parley に相当が無いため `Justify` に degrade する
        // (最終行は start-aligned のまま — 既知の差分として doc に残す)。
        TextAlign::JustifyAll => Alignment::Justify,
        // `MatchParent` は computed 層到達前に解決済のはず
        // (`raikiri_style::computed::ComputedValues::text_align` doc 参照)。
        // defensive fallback として `Start` (initial value) に倒す。
        TextAlign::MatchParent => Alignment::Start,
        // `#[non_exhaustive]` 将来 variant への fail-closed fallback。
        _ => Alignment::Start,
    }
}

/// taffy の `compute_root_layout` 後に各 Text の parley `Layout` を
/// containing block 幅基準で re-align する。
///
/// # なぜ preshape 時ではなくここか
///
/// [`preshape_text`] は taffy より前に走り、使える幅は `page_box.width`
/// のみ。`Center` 等をそこで素朴に渡すと、狭い containing block 内の text が
/// ページ幅基準で中央寄せされ、大きくズレる。
/// taffy 後に親 box の確定幅 (`unrounded_layout.size.width`) を
/// containing 幅として `break_all_lines(Some(w))` + `align(...)` し直すことで、
/// 正しい幅基準の offset を glyph run に bake する。
/// `Start` は幅に依存しないため skip する (preshape のまま正しい)。
///
/// # flex line box の child は対象外
///
/// 親が [`establish_minimal_line_boxes`] で flex 化された container
/// (`IS_INLINE_ROOT`) の場合、その Text は line box 上の flex item であり、
/// 中央寄せは container 側の `justify_content: Center` が担う。
/// ここで親幅基準に parley re-align すると各 piece が親全幅基準で中央に
/// 寄り、互いに重なってしまうため、明示的に skip する。
/// 親幅ではなく自身の box 幅で寄せても offset 0 の no-op にしかならないが、
/// 意図を明示するため skip 側に倒す。
///
/// # 既知の限界
///
/// - 親の `size.width` をそのまま containing 幅にする。padding / border の
///   content-box 減算はしない (px length のみ減算すべきだが、現状は未対応 —
///   padding 付き narrow container では中央がわずかにずれる)。
/// - re-break で行数が変わる場合 (narrow container 内の長文)、text の高さが
///   変わるが taffy box は更新しない (sibling の y が stale のまま)。
///   中央寄せの主 target である短文・単一行では高さ不変のため無害。
///   長文 wrap の完全な整合は inline formatting context 全体の再設計時に扱う。
/// - `Start` / `End` の論理→物理解決は自要素の `cv.direction` で行う。
///   parley public API に base direction を渡す口が無いため、parley 側の
///   `Start` / `End` には頼らず
///   `Left` / `Right` に解決してから渡す。`dir=rtl` 属性は対象外
///   (`direction` CSS のみ — dir 属性→direction 反映は Epic 3 領域)。
/// - `text-indent` の `%` は親の border-box 幅基準で解決する (content-box
///   減算なし — 上記第 1 bullet と同じ近似)。flex line box の child の
///   indent は未対応 (box 測定との乖離のため skip)。
///
/// tab-stop 基準の block-container 祖先を探す (CSS Text 3 §4.2)。
///
/// `Inline` / `Contents` は透過して登る。`Flex` / `Grid` 等の
/// 非 block-container で止まった場合・root 到達の場合は None
/// (caller は自 font に倒す fail-safe)。
fn nearest_block_container(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> Option<usize> {
    let mut cur = parent_of.get(idx).copied().flatten();
    while let Some(a) = cur {
        if doc.nodes[a].kind() != NodeKind::Element {
            cur = parent_of.get(a).copied().flatten();
            continue;
        }
        match cascade.computed[a].display {
            DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::ListItem => {
                return Some(a);
            }
            DisplayValue::Inline | DisplayValue::Contents => {
                cur = parent_of.get(a).copied().flatten();
            }
            _ => return None,
        }
    }
    None
}

/// CSS collapsing 対象の空白集合 (space / tab / LF / FF / CR)。
///
/// `char::is_whitespace` は NBSP 等も含むが、NBSP は collapse しないため
/// ここでは使わない (CSS Text 3 §4.1 準拠の近似)。
fn is_collapsible_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// sibling が block 内の先行 content になるか (CSS Text 3 §8.1 先頭行判定用)。
///
/// - Text: collapse 系 white-space で空白のみ → 無視 (行を占めない)。
///   preserve 系では空白も行を占めるため content。空 string は常に無視。
/// - `br` → 常に content (forced break → 後続は 2 行目以降)。
/// - `display: none` → 無視。replaced (`img` 等) → content。
///   その他: 子無し → 無視、子持ち → content。
///   (空 inline の過剰除外は受容する近似 — doc に明記。)
fn is_block_content(doc: &Document, cascade: &CascadeResult, sib: usize) -> bool {
    match &doc.nodes[sib].data {
        crate::node::NodeData::Text(t) => {
            if t.text_content.is_empty() {
                return false;
            }
            match cascade.computed[sib].white_space {
                WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine => {
                    !t.text_content.chars().all(is_collapsible_ws)
                }
                _ => true,
            }
        }
        crate::node::NodeData::Element(_) => {
            let tag = doc.nodes[sib].tag_name().unwrap_or("");
            if tag.eq_ignore_ascii_case("br") {
                return true;
            }
            if cascade.computed[sib].display == DisplayValue::None {
                return false;
            }
            if doc.nodes[sib].children.is_empty() {
                return matches!(
                    tag.to_ascii_lowercase().as_str(),
                    "img"
                        | "input"
                        | "video"
                        | "canvas"
                        | "iframe"
                        | "embed"
                        | "object"
                        | "textarea"
                        | "select"
                        | "button"
                        | "hr"
                );
            }
            true
        }
        _ => false,
    }
}

/// その Text node が属する block の先頭 formatted line を開始するか
/// (CSS Text 3 §8.1 — `text-indent` は each-line 無しでは先頭行のみ)。
///
/// per-Text-node preshape では「node の先頭行」と「block の先頭行」が一致
/// しない (`<br>` 後・inline 分割・nested block 混在 — length-002 が pin)。
/// nearest block-container 祖先まで登りながら先行 sibling を scan し、
/// content が 1 つでもあれば false。
/// block 祖先が無い場合は true (fail-safe — 従来挙動を維持)。
/// Maps computed hanging/each-line flags plus node line position to parley
/// [`IndentOptions`] (CSS Text 3 §8.1).
///
/// Returns `None` when the node must not indent at all. Position semantics:
/// - [`LineStart::MidLine`] (mid-line inline split): never indent — parley
///   cannot know the layout starts mid-line, so any amount would shift
///   already-placed content.
/// - [`LineStart::BlockStart`]: the layout's first line is the block's first
///   line — pass the flags through unchanged (parley resolves first/wrap/
///   hard-break lines itself, including hanging+each-line combined via XOR).
/// - [`LineStart::AfterBreak`]: every layout line is a non-first block line.
///   basic skips; each-line and combined pass through unchanged (exact per
///   parley scope-line semantics); hanging-only maps to
///   `{ each_line: true, hanging: false }`, which is exact for single-line
///   and post-hard-break lines — soft-wrapped continuations inside the node
///   are missed (no parley option indents all lines unconditionally).
///   Documented approximation; the common test shape (short lines) is exact.
fn indent_options_for_node(
    hanging: bool,
    each_line: bool,
    start: LineStart,
) -> Option<IndentOptions> {
    match (hanging, each_line, start) {
        (_, _, LineStart::MidLine) => None,
        (false, false, LineStart::BlockStart) => Some(IndentOptions::default()),
        (false, false, LineStart::AfterBreak) => None,
        (false, true, _) => Some(IndentOptions {
            each_line: true,
            hanging: false,
        }),
        (true, false, LineStart::BlockStart) => Some(IndentOptions {
            each_line: false,
            hanging: true,
        }),
        (true, false, LineStart::AfterBreak) => Some(IndentOptions {
            each_line: true,
            hanging: false,
        }),
        (true, true, _) => Some(IndentOptions {
            each_line: true,
            hanging: true,
        }),
    }
}

/// 行頭位置の分類 (leading trim 用)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineStart {
    /// block 先頭。
    BlockStart,
    /// `<br>` / preserved `\n` / 先行 block の直後。
    AfterBreak,
    /// 上記以外 (行中)。
    MidLine,
}

/// Text node が preserved な強制 break (`\n`) を含むか。
///
/// preserve するのは Pre / PreWrap / BreakSpaces / PreLine。
/// Normal / Nowrap 下の `\n` は space 化されるため break ではない。
fn has_preserved_break(cascade: &CascadeResult, idx: usize, text: &str) -> bool {
    if !text.contains('\n') {
        return false;
    }
    !matches!(
        cascade.computed[idx].white_space,
        WhiteSpace::Normal | WhiteSpace::Nowrap
    )
}

/// 行頭位置を後ろ向き scan で求める (leading trim 用)。
///
/// block 祖先まで登りながら先行 sibling を逆順 scan:
/// - collapse 系の空白のみ text / 空 / `display: none` → 透過。
/// - preserved `\n` を含む text / `br` / 先行 block 要素 → [`LineStart::AfterBreak`]。
/// - その他 content → [`LineStart::MidLine`]。
/// - block 先頭 (or root) 到達 → [`LineStart::BlockStart`]。
fn line_start_pos(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> LineStart {
    let block = nearest_block_container(doc, cascade, parent_of, idx);
    let mut cur = idx;
    loop {
        let Some(p) = parent_of.get(cur).copied().flatten() else {
            return LineStart::BlockStart;
        };
        if let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == cur) {
            for &sib in doc.nodes[p].children[..pos].iter().rev() {
                match &doc.nodes[sib].data {
                    crate::node::NodeData::Text(t) => {
                        if has_preserved_break(cascade, sib, &t.text_content) {
                            return LineStart::AfterBreak;
                        }
                        if is_block_content(doc, cascade, sib) {
                            return LineStart::MidLine;
                        }
                    }
                    crate::node::NodeData::Element(_) => {
                        let tag = doc.nodes[sib].tag_name().unwrap_or("");
                        if tag.eq_ignore_ascii_case("br") {
                            return LineStart::AfterBreak;
                        }
                        match cascade.computed[sib].display {
                            DisplayValue::Block
                            | DisplayValue::InlineBlock
                            | DisplayValue::ListItem => {
                                return LineStart::AfterBreak;
                            }
                            DisplayValue::None => {}
                            _ => {
                                if is_block_content(doc, cascade, sib) {
                                    return LineStart::MidLine;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if Some(p) == block {
            return LineStart::BlockStart;
        }
        cur = p;
    }
}

/// 後続が block 終端・`<br>`・preserved `\n`・後続 block のいずれか
/// (trailing trim 用)。前向き scan、分類は [`line_start_pos`] と対称。
fn trail_trim(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> bool {
    let block = nearest_block_container(doc, cascade, parent_of, idx);
    let mut cur = idx;
    loop {
        let Some(p) = parent_of.get(cur).copied().flatten() else {
            return true;
        };
        if let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == cur) {
            for &sib in doc.nodes[p].children[pos + 1..].iter() {
                match &doc.nodes[sib].data {
                    crate::node::NodeData::Text(t) => {
                        if has_preserved_break(cascade, sib, &t.text_content) {
                            return true;
                        }
                        if is_block_content(doc, cascade, sib) {
                            return false;
                        }
                    }
                    crate::node::NodeData::Element(_) => {
                        let tag = doc.nodes[sib].tag_name().unwrap_or("");
                        if tag.eq_ignore_ascii_case("br") {
                            return true;
                        }
                        match cascade.computed[sib].display {
                            DisplayValue::Block
                            | DisplayValue::InlineBlock
                            | DisplayValue::ListItem => {
                                return true;
                            }
                            DisplayValue::None => {}
                            _ => {
                                if is_block_content(doc, cascade, sib) {
                                    return false;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if Some(p) == block {
            return true;
        }
        cur = p;
    }
}

/// white-space phase 1 collapsing (CSS Text 3 §4.1)。
///
/// - Pre / PreWrap / BreakSpaces → 無変換 (tab 展開は [`preshape_text`] で別途行う)。
/// - Normal / Nowrap: `\t \n \f \r` → space、run collapse、行頭/行末 trim。
/// - PreLine: `\n` 保持 (forced break)、他は Normal と同じ。
/// - leading trim は `start != MidLine`、trailing trim は `trim_end`。
///
/// 対象外 (doc 明記): segment-break 前後の CJK space 除去、PreLine の
/// hanging trailing space、`overflow-wrap` との相互作用。
fn collapse_ws(
    text: &str,
    ws: WhiteSpace,
    start: LineStart,
    trim_end: bool,
) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
            return Cow::Borrowed(text);
        }
        _ => {}
    }
    let keep_nl = ws == WhiteSpace::PreLine;
    // 速径: 変換対象が無ければ borrow のまま返す。
    let needs = text.chars().any(|c| match c {
        '\t' | '\x0C' | '\r' => true,
        '\n' => !keep_nl,
        ' ' => true,
        _ => false,
    });
    if !needs {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    let mut at_start = true;
    // 先頭 run の扱い: BlockStart / AfterBreak では捨て、MidLine では 1 space。
    let mut drop_leading = start != LineStart::MidLine;
    for c in text.chars() {
        if c == '\n' && keep_nl {
            // PreLine の preserved break: pending を flush して改行を置き、
            // 改行後は常に行頭扱い (start-of-line run 除去 — CSS Text 3 §4.1.2)。
            // break 直前 trailing の hanging 再現は対象外 (1 space 置く)。
            if pending_space && !at_start {
                out.push(' ');
            }
            out.push('\n');
            pending_space = false;
            at_start = true;
            drop_leading = true;
            continue;
        }
        let is_ws = matches!(c, '\t' | '\x0C' | '\r' | '\n' | ' ');
        if is_ws {
            if !at_start || !drop_leading {
                pending_space = true;
            }
            continue;
        }
        if pending_space && (!at_start || !drop_leading) {
            out.push(' ');
        }
        pending_space = false;
        at_start = false;
        out.push(c);
    }
    // 末尾 run: `trim_end` (block 終端 / break 直前) のとき捨て、
    // それ以外は 1 space。
    if pending_space && !trim_end {
        out.push(' ');
    }
    Cow::Owned(out)
}

/// Prepare font-metric `text-indent: ch` values before Taffy measures leaves.
///
/// Parley first shapes against the page width, but Taffy may later ask a text
/// leaf for an intrinsic size at a narrower containing width. The prepared
/// metric and flags live on `TextData`; `taffy_impl` reapplies them for each
/// width probe and rebreaks the existing layout before returning its height.
pub(crate) fn prepare_text_indent_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
) {
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &child in &doc.nodes[idx].children.clone() {
            if child < parent_of.len() {
                parent_of[child] = Some(idx);
            }
        }
    }
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for idx in 0..doc.nodes.len() {
        let Some(text) = doc.nodes[idx].data.as_text_mut() else {
            continue;
        };
        text.text_indent_px = None;
        text.text_indent_hanging = false;
        text.text_indent_each_line = false;
        text.text_indent_rebreak = false;
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        let cv = &cascade.computed[idx];
        let Some(parent_idx) = parent_of[idx] else {
            continue; // cov:ignore: in-document text nodes always have a parent.
        };
        if cv.text_indent_ch_factor.is_none() {
            continue;
        }
        let needs_line_break_property = matches!(
            cv.word_break,
            WordBreak::BreakAll | WordBreak::KeepAll | WordBreak::BreakWord
        ) || !matches!(cv.overflow_wrap, OverflowWrap::Normal)
            || matches!(cv.white_space, WhiteSpace::BreakSpaces);
        let has_forced_break = doc.nodes[parent_idx]
            .children
            .iter()
            .any(|&child| is_forced_line_break(doc, child, cascade));
        let parent_is_inline_root = doc.nodes[parent_idx]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT);
        let has_modifier = cv.text_indent_hanging || cv.text_indent_each_line;
        let parent_is_block_container = matches!(
            cascade.computed[parent_idx].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        );
        let has_other_in_document_child = has_modifier
            && doc.nodes[parent_idx]
                .children
                .iter()
                .any(|&child| child != idx && doc.nodes[child].is_in_document());
        if (has_modifier && (!parent_is_block_container || has_other_in_document_child))
            || (parent_is_inline_root
                && (has_modifier || (!needs_line_break_property && has_forced_break)))
            || (cascade.computed[parent_idx].width_ch.is_some()
                && (has_modifier || !needs_line_break_property))
        {
            // Modifier line scope and a `ch` containing width still use the
            // established post-Taffy path for inline roots and width-aware
            // boxes. The pre-Taffy bridge is limited to direct, finite-width
            // text runs where its measured line break is unambiguous.
            continue;
        }
        let layout_max_advance = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(text) => text
                .text_layout
                .as_ref()
                .map(|layout| layout.layout_max_advance()),
            _ => None, // cov:ignore: the preceding as_text_mut guard makes this arm unreachable.
        };
        let preserve_wide_body_run = layout_max_advance
            .is_some_and(|advance| advance > max_advance + 0.01)
            && is_leading_body_text(doc, Some(parent_idx), idx)
            && !needs_line_break_property
            && matches!(cv.text_align, TextAlign::Start | TextAlign::Left);
        if preserve_wide_body_run {
            // Match `realign_text_after_layout`: this deliberately keeps the
            // page-width run untouched, including its indent metadata.
            continue;
        }
        let Some(options) = indent_options_for_node(
            cv.text_indent_hanging,
            cv.text_indent_each_line,
            line_start_pos(doc, cascade, &parent_of, idx),
        ) else {
            continue;
        };
        let Some(indent) = measured_text_indent_px(cv, fonts, layout_cx, &mut ch_probes) else {
            continue; // cov:ignore: only computed ch provenance reaches this point.
        };
        let hanging = options.hanging;
        let each_line = options.each_line;
        // Rebreak direct metric-backed runs before Taffy so their line count
        // contributes to the leaf height. Guarded modifier and `width: ch`
        // shapes remain on the post-Taffy path above.
        let rebreak =
            !matches!(cv.white_space, WhiteSpace::Nowrap) && cv.text_wrap != TextWrapMode::Nowrap;
        let text = doc.nodes[idx]
            .data
            .as_text_mut()
            .expect("text node remains text during layout preparation");
        text.text_indent_px = Some(indent);
        text.text_indent_hanging = hanging;
        text.text_indent_each_line = each_line;
        text.text_indent_rebreak = rebreak;
        if let Some(layout) = text.text_layout.as_mut() {
            layout.set_text_indent(indent, options);
            if rebreak && max_advance.is_finite() && max_advance > 0.0 {
                layout.break_all_lines(Some(max_advance));
            }
        } // cov:ignore: the no-layout branch is defensive for pre-shaped callers.
    }
}

pub(crate) fn prepare_ch_box_values_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) {
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let mut measure = |provenance: &Option<ChLengthProvenance>| {
            provenance.as_ref().map(|provenance| {
                measured_ch_length_px(
                    provenance.factor,
                    Some(&provenance.font),
                    cv,
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                )
            })
        };
        let width = measure(&cv.width_ch).map(|value| value.max(0.0));
        let height = measure(&cv.height_ch).map(|value| value.max(0.0));
        let padding = (
            measure(&cv.padding_ch.top).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.right).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.bottom).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.left).map(|value| value.max(0.0)),
        );
        let margin = (
            measure(&cv.margin_ch.top),
            measure(&cv.margin_ch.right),
            measure(&cv.margin_ch.bottom),
            measure(&cv.margin_ch.left),
        );
        let style = &mut doc.nodes[idx].style;
        if let Some(width) = width {
            style.size.width = Dimension::length(width);
        }
        if let Some(height) = height {
            style.size.height = Dimension::length(height);
        }
        if let Some(top) = padding.0 {
            style.padding.top = LengthPercentage::length(top);
        }
        if let Some(right) = padding.1 {
            style.padding.right = LengthPercentage::length(right);
        }
        if let Some(bottom) = padding.2 {
            style.padding.bottom = LengthPercentage::length(bottom);
        }
        if let Some(left) = padding.3 {
            style.padding.left = LengthPercentage::length(left);
        }
        if let Some(top) = margin.0 {
            style.margin.top = LengthPercentageAuto::length(top);
        }
        if let Some(right) = margin.1 {
            style.margin.right = LengthPercentageAuto::length(right);
        }
        if let Some(bottom) = margin.2 {
            style.margin.bottom = LengthPercentageAuto::length(bottom);
        }
        if let Some(left) = margin.3 {
            style.margin.left = LengthPercentageAuto::length(left);
        }
    }
}

// cov:ignore: exercised by resource-enabled ignored WPT reftests; default coverage has no sparse asset run.
pub(crate) fn realign_inline_replaced_children(doc: &mut Document, cascade: &CascadeResult) {
    for parent_id in 0..doc.nodes.len() {
        if !doc.nodes[parent_id]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
        {
            continue;
        }
        let has_replaced_child = doc.nodes[parent_id].children.iter().any(|&child| {
            doc.nodes[child].is_in_document() && doc.nodes[child].tag_name() == Some("img")
        });
        if !has_replaced_child {
            continue;
        }
        let parent_width = doc.nodes[parent_id].unrounded_layout.size.width.max(0.0);
        if !parent_width.is_finite() || parent_width <= 0.0 {
            continue;
        }
        let line_height = line_height_px(&cascade.computed[parent_id]).max(0.0);
        if line_height <= 0.0 || !line_height.is_finite() {
            continue;
        }
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut line_height_used = line_height;
        for &child in &doc.nodes[parent_id].children.clone() {
            if !doc.nodes[child].is_in_document() {
                continue;
            }
            if doc.nodes[child].tag_name() == Some("br") {
                x = 0.0;
                y += line_height_used;
                line_height_used = line_height;
                continue;
            }
            let is_collapsible_whitespace = matches!(
                &doc.nodes[child].data,
                NodeData::Text(text)
                    if text.text_content.chars().all(is_collapsible_ws)
            );
            let is_nonbreaking_space_run = matches!(
                &doc.nodes[child].data,
                NodeData::Text(text)
                    if !text.text_content.is_empty()
                        && text.text_content.chars().all(|ch| ch == '\u{00a0}')
            );
            let (width, height) = if is_collapsible_whitespace || is_nonbreaking_space_run {
                (
                    doc.nodes[child].unrounded_layout.size.width.max(
                        style_dimension_length(doc.nodes[child].style.size.width).unwrap_or(0.0),
                    ),
                    0.0,
                )
            } else if doc.nodes[child].tag_name() == Some("img") {
                (
                    doc.nodes[child].unrounded_layout.size.width.max(0.0),
                    doc.nodes[child].unrounded_layout.size.height.max(0.0),
                )
            } else {
                continue;
            };
            if width <= 0.0 || !width.is_finite() {
                continue;
            }
            if x > 0.0 && x + width > parent_width + 0.01 {
                x = 0.0;
                y += line_height_used;
                line_height_used = line_height;
                if is_collapsible_whitespace {
                    continue;
                }
            }
            if is_collapsible_whitespace && x == 0.0 {
                // CSS-collapsible leading whitespace is removed at a new line.
                // U+00A0 is not collapsible and keeps its measured advance.
                continue;
            }
            doc.nodes[child].unrounded_layout.location.x = x;
            doc.nodes[child].unrounded_layout.location.y = y;
            x += width;
            line_height_used = line_height_used.max(height);
        }
        let used_height = if x > 0.0 || y == 0.0 {
            y + line_height_used
        } else {
            y
        };
        doc.nodes[parent_id].unrounded_layout.size.height = used_height;
    }
}

// Text indentation is otherwise applied to TextData layouts, so an empty
// inline block has no text layout to receive the first-line offset. Keep this
// post-layout bridge narrow: only one empty inline-block in a horizontal LTR
// block, with optional collapsible whitespace, no modifiers or forced breaks,
// and normal-flow positioning can be offset without reflowing a line.
pub(crate) fn realign_single_empty_inline_block_indent(
    doc: &mut Document,
    cascade: &CascadeResult,
) {
    for parent_id in 0..doc.nodes.len() {
        let cv = &cascade.computed[parent_id];
        if !matches!(cv.display, DisplayValue::Block | DisplayValue::InlineBlock) {
            continue;
        }
        if cv.direction != Direction::Ltr
            || cv.writing_mode != WritingMode::HorizontalTb
            || cv.text_indent_hanging
            || cv.text_indent_each_line
            || cv.text_indent_ch_factor.is_some()
            || !matches!(
                cv.text_align,
                TextAlign::Start | TextAlign::Left | TextAlign::Right
            )
        {
            continue;
        }

        let mut inline_block = None;
        let mut unsupported_content = false;
        for &child in &doc.nodes[parent_id].children {
            if !doc.nodes[child].is_in_document() {
                continue;
            }
            if is_forced_line_break(doc, child, cascade) {
                unsupported_content = true;
                break;
            }
            match &doc.nodes[child].data {
                NodeData::Text(text)
                    if matches!(
                        cascade.computed[child].white_space,
                        WhiteSpace::Normal | WhiteSpace::Nowrap
                    ) && text.text_content.chars().all(is_css_white_space) => {}
                NodeData::Element(_)
                    if cascade.computed[child].display == DisplayValue::InlineBlock
                        && cascade.computed[child].float == FloatValue::None
                        && matches!(
                            cascade.computed[child].position,
                            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                        )
                        && doc.nodes[child].children.is_empty() =>
                {
                    if inline_block.replace(child).is_some() {
                        unsupported_content = true;
                        break;
                    }
                }
                _ => {
                    unsupported_content = true;
                    break;
                }
            }
        }
        if unsupported_content {
            continue;
        }
        let Some(inline_block) = inline_block else {
            continue;
        };

        let content_width = doc.nodes[parent_id].unrounded_layout.content_box_width();
        if !content_width.is_finite() || content_width < 0.0 {
            continue;
        }
        let indent = bounded_text_indent_amount(cv.text_indent, content_width, None);
        if !indent.is_finite() || indent == 0.0 {
            continue;
        }
        doc.nodes[inline_block].unrounded_layout.location.x += indent;
    }
}

/// Find the nearest synthetic inline formatting root for a text node.
fn inline_root_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_id: usize,
) -> Option<usize> {
    let mut ancestor = parent_of[text_id];
    while let Some(id) = ancestor {
        if doc.nodes[id].flags.contains(NodeFlags::IS_INLINE_ROOT) {
            return Some(id);
        }
        ancestor = parent_of[id];
    }
    None
}

/// Return a node's horizontal position in the document coordinate space.
fn absolute_layout_x(doc: &Document, parent_of: &[Option<usize>], mut id: usize) -> f32 {
    let mut x = 0.0;
    while let Some(parent) = parent_of[id] {
        x += doc.nodes[id].unrounded_layout.location.x;
        id = parent;
    }
    x
}

pub(crate) fn realign_text_after_layout(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) {
    // parent map (arena に parent pointer が無いため children から逆引き)。
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for (idx, parent) in parent_of.iter().enumerate() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        let mut ancestor = *parent;
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        let mut inside_multicol = false;
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        while let Some(ancestor_id) = ancestor {
            if !matches!(
                cascade.computed[ancestor_id].column_count,
                ColumnCountValue::Auto
            ) || !matches!(
                cascade.computed[ancestor_id].column_width,
                ComputedColumnWidth::Auto
            ) {
                inside_multicol = true;
                break;
            }
            ancestor = parent_of[ancestor_id];
        }
        // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
        if inside_multicol
            // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
            && matches!(
                &doc.nodes[idx].data,
                NodeData::Text(text) if text.multicol_fragments.is_some()
            )
        // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
        {
            // Fragment ranges are already width-constrained and their line
            // offsets are consumed by the multicol paint path. Nested inline
            // text still needs the ordinary post-Taffy alignment pass.
            continue;
        }
        // 論理値 (`start` / `end`) は自要素の `direction` で物理値に解決する
        // (CSS Text 3 §6.1)。parley の `Start` / `End` は content-inferred
        // bidi に委ねられるため、RTL では誤った側に寄る
        // (text-align-end-001 の regress で実測)。
        // `Start` + LTR のみ skip (preshape のまま正しい — 再 break による
        // wrap 変化の regress を避ける)。
        let cv = &cascade.computed[idx];
        let parley_align = match text_align_to_parley(cv.text_align) {
            Alignment::Start if cv.direction == Direction::Rtl => Alignment::Right,
            Alignment::End if cv.direction == Direction::Rtl => Alignment::Left,
            a => a,
        };
        // `text-justify: none` disables justification (CSS Text 3 §6.2):
        // combined with `text-align: justify` it falls back to the start
        // edge (direction-aware).
        let mut align = parley_align;
        if cv.text_justify == TextJustify::None && align == Alignment::Justify {
            align = match cv.direction {
                Direction::Rtl => Alignment::Right,
                _ => Alignment::Start,
            };
        }
        // `text-indent` (CSS Text 3 §8.1): length plus hanging/each-line
        // flags, resolved against the containing block width for `%`
        // (unknown at preshape time — same width-basis problem as align).
        // The indent applies per node position (see `indent_options_for_node`
        // doc): MidLine never, BlockStart always, AfterBreak per flags.
        let nonzero_indent = match cv.text_indent {
            ComputedLengthPercentage::Px(px) => px != 0.0,
            ComputedLengthPercentage::Percent(p) => p != 0.0,
        };
        let indent_options = if nonzero_indent {
            indent_options_for_node(
                cv.text_indent_hanging,
                cv.text_indent_each_line,
                line_start_pos(doc, cascade, &parent_of, idx),
            )
        } else {
            None
        };
        let measured_indent = measured_text_indent_px(cv, fonts, layout_cx, &mut ch_probes);
        // preshape は page 幅で break するため、narrow container 内の text の
        // 折り返しは container 幅に整合しない (slice 1b で
        // 実測: length-001 の 3-line article が single-line のまま残る)。
        // 全 node re-break の実験は nowrap-001 の pinned regress を起こしたため
        // revert した (当該 experiment は別途 full-baseline 判定が必要 —
        // the preceding comment参照)。したがって re-break は
        // indent 付き / 非 Start の node のみに限定する。
        // All non-flex text re-breaks against the containing width here
        // preshape only knows the page width, so
        // narrow-container wrapping would otherwise never apply. `nowrap`
        // nodes are handled by the branch below without re-breaking.
        // `Start` alignment after re-break is a no-op offset-wise; only the
        // wrap points change.
        // `Justify` / `End` 等も同じ幅基準問題を持つため同じ経路で扱うが、
        // 本 goal の主 target は `Center`。他値もここで正しい幅基準になる。
        let Some(parent_idx) = *parent else {
            continue;
        };
        // flex line box の child は container 側の justify に委ねる (上記 doc)。
        // indent も同様に skip する — flex item の幅は taffy が indent 無し
        // layout から測っており、ここで indent を付けると box と run が乖離
        // する (既知の限界として doc に残す)。
        let needs_line_break_property = matches!(
            cv.word_break,
            WordBreak::BreakAll | WordBreak::KeepAll | WordBreak::BreakWord
        ) || !matches!(cv.overflow_wrap, OverflowWrap::Normal)
            || matches!(cv.white_space, WhiteSpace::BreakSpaces);
        let has_forced_break = doc.nodes[parent_idx]
            .children
            .iter()
            .any(|&child| is_forced_line_break(doc, child, cascade));
        if doc.nodes[parent_idx]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
            && !inside_multicol
            && !needs_line_break_property
            && (cv.text_indent_ch_factor.is_none()
                || cv.text_indent_hanging
                || cv.text_indent_each_line
                || has_forced_break)
        {
            continue;
        }
        let containing_width = doc.nodes[parent_idx].unrounded_layout.size.width;
        if !containing_width.is_finite() || containing_width <= 0.0 {
            continue;
        }
        let preserve_wide_body_run = !inside_multicol
            && is_leading_body_text(doc, Some(parent_idx), idx)
            && matches!(cv.text_align, TextAlign::Start | TextAlign::Left);
        let inline_continuation = inline_root_for_text(doc, &parent_of, idx).and_then(|root| {
            let root_cv = &cascade.computed[root];
            if root == parent_idx
                || cv.direction != Direction::Ltr
                || root_cv.direction != Direction::Ltr
                || root_cv.writing_mode != WritingMode::HorizontalTb
            {
                return None;
            }
            let root_x = absolute_layout_x(doc, &parent_of, root);
            let node_x = absolute_layout_x(doc, &parent_of, idx);
            let prefix = node_x - root_x;
            let width = doc.nodes[root].unrounded_layout.size.width;
            if !prefix.is_finite() || !width.is_finite() || prefix <= 0.0 || width <= prefix {
                return None;
            }
            Some((width - prefix, root_x - node_x))
        });
        let Some(layout) = doc.nodes[idx]
            .data
            .as_text_mut()
            .and_then(|t| t.text_layout.as_mut())
        else {
            continue;
        };
        if let Some((available, continuation_offset)) = inline_continuation {
            let should_rebreak = available.is_finite()
                && available > 0.0
                && available + f32::EPSILON < layout.width();
            if should_rebreak {
                layout.break_all_lines(Some(available));
                layout.align(Alignment::Start, AlignmentOptions::default());
                let line_count = layout.len();
                if let NodeData::Text(text) = &mut doc.nodes[idx].data {
                    text.text_line_offsets = Some(
                        (0..line_count)
                            .map(|line| if line == 0 { 0.0 } else { continuation_offset })
                            .collect(),
                    );
                }
                continue;
            }
        }
        // A direct body text run may intentionally overflow the page content
        // width when the paged containing block carries a margin.  Preserve
        // the page-width shaping used by the paged bridge instead of
        // re-breaking a one-line run to the narrower body box.
        if preserve_wide_body_run
            && layout.width() > containing_width + 0.01
            && !needs_line_break_property
        {
            continue;
        }
        // No-wrap (`white-space: nowrap` or `text-wrap: nowrap`): preshape
        // already broke without a width cap, so re-breaking here would wrap.
        // Apply indent and align onto the preshaped single line instead.
        // Single-line Justify is a parley no-op (last line excluded), which
        // matches `text-align: justify` under nowrap.
        if cv.white_space == WhiteSpace::Nowrap || cv.text_wrap == TextWrapMode::Nowrap {
            if let Some(options) = indent_options {
                let amount =
                    bounded_text_indent_amount(cv.text_indent, containing_width, measured_indent);
                layout.set_text_indent(amount, options);
            }
            layout.align(align, AlignmentOptions::default());
            continue;
        }
        if let Some(options) = indent_options {
            let amount =
                bounded_text_indent_amount(cv.text_indent, containing_width, measured_indent);
            layout.set_text_indent(amount, options);
        }
        layout.break_all_lines(Some(containing_width));
        // `text-align-last` (CSS Text 3 §6.1): 明示 last 値の適用は対象外。
        // parley の `align(Justify)` は最終行 (`BreakReason::None`) を
        // 意図的に除外するため (parley alignment.rs 実測)、single-line を含む
        // あらゆる last-line-only の justify/align は public API では再現
        // できない。`text_align_last` の cascade 配線 (parse/wire/inherit) は
        // 将来の parley 対応に備えて維持する。`MatchParent` の cascade
        // 未解決も同様に将来対応 (現状 `auto` 扱いの近似は reader 側で行わない —
        // 何もしないのが正しい)。
        layout.align(align, AlignmentOptions::default());
    }

    // Rebreaking text after Taffy can increase an inline child's line count.
    // Propagate that used height through auto-sized ancestors so multicolumn
    // item backgrounds and the enclosing block cover the rebroken run.
    // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text || !doc.nodes[idx].is_in_document() {
            continue;
        }
        if matches!(
            &doc.nodes[idx].data,
            NodeData::Text(text) if text.multicol_fragments.is_some()
        ) {
            continue;
        }
        let Some(text_height) = doc.nodes[idx].text_layout().map(|layout| layout.height()) else {
            continue;
        };
        if !text_height.is_finite() || text_height <= 0.0 {
            continue;
        }
        doc.nodes[idx].unrounded_layout.size.height =
            doc.nodes[idx].unrounded_layout.size.height.max(text_height);
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }

    // Taffy does not include floated descendants in an auto-height
    // containing block's used height. Propagate the bottom edge of supported
    // left/right floats through auto-height ancestors, including the
    // containing-block border. This keeps following flow from moving upward
    // after a fragmented flex item with a float descendant.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element
            || !doc.nodes[idx].is_in_document()
            || !matches!(
                cascade.computed[idx].float,
                FloatValue::Left | FloatValue::Right
            )
        {
            continue;
        }
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }
}
/// Collapse adjacent block margins inside a non-replaced inline box.
///
/// CSS 2.1 §9.2.1.1 splits an inline containing block around block-level
/// children. Whitespace-only text between those children is part of the
/// anonymous inline fragments, not an intervening block. Taffy's block path
/// already gives the children the right vertical order, but it does not
/// perform this block-in-inline margin collapse; keeping both margins would
/// add one extra line-height-sized gap between two adjacent block children.
/// Keep this pass narrow: only direct children of a plain `inline` wrapper and
/// only resolvable absolute margins are rewritten.
pub(crate) fn collapse_block_in_inline_margins(
    doc: &mut Document,
    idx: usize,
    cascade: &CascadeResult,
) {
    if cascade.computed[idx].display != DisplayValue::Inline {
        return;
    }

    let mut previous_block = None;
    for &child in &doc.nodes[idx].children.clone() {
        if !doc.nodes[child].is_in_document() {
            continue;
        }
        match doc.nodes[child].kind() {
            NodeKind::Text => {
                // Whitespace around a block child belongs to the anonymous
                // inline fragments and must not interrupt sibling margin
                // collapse. Any visible text does interrupt that sequence.
                if !text_of(doc, child).is_some_and(|text| text.chars().all(is_css_white_space)) {
                    previous_block = None;
                }
            }
            NodeKind::Element => {
                let display = cascade.computed[child].display;
                if display == DisplayValue::None {
                    continue;
                }
                if is_inline_element_box(cascade, child) {
                    previous_block = None;
                    continue;
                }
                if let Some(previous) = previous_block {
                    collapse_adjacent_block_margins(doc, previous, child);
                }
                previous_block = Some(child);
            }
            _ => previous_block = None,
        }
    }
}

fn collapse_adjacent_block_margins(doc: &mut Document, previous: usize, next: usize) {
    let previous_margin = doc.nodes[previous].style.margin.bottom.into_raw();
    let next_margin = doc.nodes[next].style.margin.top.into_raw();
    if previous_margin.tag() != CompactLength::LENGTH_TAG
        || next_margin.tag() != CompactLength::LENGTH_TAG
    {
        return;
    }
    let previous_margin = previous_margin.value();
    let next_margin = next_margin.value();
    if !previous_margin.is_finite() || !next_margin.is_finite() {
        return;
    }
    let collapsed = if previous_margin >= 0.0 && next_margin >= 0.0 {
        previous_margin.max(next_margin)
    } else if previous_margin <= 0.0 && next_margin <= 0.0 {
        previous_margin.min(next_margin)
    } else {
        previous_margin + next_margin
    };
    if collapsed.is_finite() {
        doc.nodes[previous].style.margin.bottom = LengthPercentageAuto::length(collapsed);
        doc.nodes[next].style.margin.top = LengthPercentageAuto::length(0.0);
    }
}

/// Return the nearest block-like inline formatting context for a node.
///
/// The nested-inline bridge must not be enabled by an autospace pair in an
/// unrelated sibling block. Low-level DOM tests do not install the HTML UA
/// stylesheet, so if no block-like ancestor is visible the highest reachable
/// ancestor is used as the conservative context fallback.
fn inline_formatting_context_root(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> usize {
    let mut current = node_id;
    let mut fallback = node_id;
    while let Some(parent) = parent_of.get(current).copied().flatten() {
        fallback = parent;
        if matches!(
            cascade.computed[parent].display,
            DisplayValue::Block
                | DisplayValue::InlineBlock
                | DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
                | DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRow
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        ) {
            return parent;
        }
        current = parent;
    }
    fallback
}

/// Return whether one inline formatting context contains an actual autospace
/// boundary. This reuses the same edge discovery and `text-autospace` option
/// handling as `preshape_text`, instead of combining character classes from
/// unrelated text nodes or ignoring custom values and language gating.
fn inline_context_has_autospace_candidate(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    context_root: usize,
) -> bool {
    for (idx, node) in doc.nodes.iter().enumerate() {
        if node.kind() != NodeKind::Text
            || !node.is_in_document()
            || inline_formatting_context_root(cascade, parent_of, idx) != context_root
            || cascade.computed[idx].text_autospace == TextAutospace::NoAutospace
            || has_vertical_writing_mode(cascade, parent_of, idx)
        {
            continue;
        }
        let NodeData::Text(text) = &node.data else {
            // cov:ignore: NodeKind::Text always carries NodeData::Text; this is a defensive match arm.
            continue;
        };
        let language = effective_language_for_text(doc, parent_of, idx);
        let before = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, -1);
        let after = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, 1);
        if !text_autospace_boxes_with_edges(
            &text.text_content,
            cascade.computed[idx].text_autospace,
            &language,
            1.0,
            before,
            after,
        )
        .is_empty()
        {
            return true;
        }
    }
    false
}

/// Vertical line-box extent contributed by a supported `vertical-align` value.
/// Positive raised offsets extend the line's ascent; negative lowered offsets
/// extend its descent. The paint-side shift is still applied to glyphs.
/// spec: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn vertical_align_linebox_extent(
    vertical_align: VerticalAlign,
    parent_font_size_px: f32,
) -> (f32, f32) {
    let shift = match vertical_align {
        VerticalAlign::Sub => parent_font_size_px / 5.0,
        VerticalAlign::Super => -(parent_font_size_px / 3.0),
        VerticalAlign::Length(Length::Px(px)) => -px,
        _ => 0.0,
    };
    if !shift.is_finite() {
        return (0.0, 0.0);
    }
    if shift < 0.0 {
        (-shift, 0.0)
    } else {
        (0.0, shift)
    }
}

/// Add synthetic line-box leading/trailing to authored padding without losing
/// a percentage component. Taffy's calc callback already resolves this mixed
/// value for bridged CSS lengths.
fn padding_with_linebox_extent(
    doc: &mut Document,
    value: ComputedLengthPercentage,
    extra: f32,
    site: &'static str,
) -> LengthPercentage {
    if extra <= 0.0 {
        return computed_length_percentage_to_taffy_length_percentage(
            value,
            site,
            &mut doc.layout_warnings,
        );
    }
    let extra = sanitize_taffy(extra, site, &mut doc.layout_warnings);
    match value {
        ComputedLengthPercentage::Px(px) => {
            LengthPercentage::length(sanitize_taffy(px + extra, site, &mut doc.layout_warnings))
        }
        ComputedLengthPercentage::Percent(percent) => {
            let percent = sanitize_taffy(percent, site, &mut doc.layout_warnings);
            let value = CalcLengthPercentage { percent, px: extra };
            doc.calc_values.push(std::sync::Arc::new(value));
            let pointer = doc
                .calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            LengthPercentage::calc(pointer)
        }
    }
}

pub(crate) fn establish_minimal_line_boxes(doc: &mut Document, cascade: &CascadeResult) {
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    let mut autospace_candidates_by_root = HashMap::new();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // A multicolumn container establishes fragmentainers rather than the
        // synthetic single flex line used by ordinary inline roots.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        if !matches!(cascade.computed[idx].column_count, ColumnCountValue::Auto)
            // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
            || !matches!(
                cascade.computed[idx].column_width,
                ComputedColumnWidth::Auto
            )
        // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
        {
            doc.nodes[idx].flags.remove(NodeFlags::IS_INLINE_ROOT);
            continue;
        }
        collapse_block_in_inline_margins(doc, idx, cascade);
        let has_autospace_candidate = if cascade.computed[idx].display == DisplayValue::Inline {
            let context_root = inline_formatting_context_root(cascade, &parent_of, idx);
            *autospace_candidates_by_root
                .entry(context_root)
                .or_insert_with(|| {
                    inline_context_has_autospace_candidate(doc, cascade, &parent_of, context_root)
                })
        } else {
            false
        };
        let qualifies = qualifies_for_minimal_line_box(doc, idx, cascade, has_autospace_candidate);
        doc.nodes[idx]
            .flags
            .set(NodeFlags::IS_INLINE_ROOT, qualifies);
        if !qualifies {
            continue;
        }
        // display:none な child も含む — taffy はそのような child を
        // Display::None として layout tree から丸ごと除外するため、
        // flex_grow/flex_shrink を check することに実害は無い (無駄では
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
        let container_cv = &cascade.computed[idx];
        let has_ch_indent = container_cv.text_indent_ch_factor.is_some()
            && !container_cv.text_indent_hanging
            && !container_cv.text_indent_each_line;
        let mut linebox_leading = 0.0_f32;
        let mut linebox_trailing = 0.0_f32;
        for &child in &participating_children {
            if cascade.computed[child].display == DisplayValue::None {
                continue;
            }
            let (leading, trailing) = vertical_align_linebox_extent(
                cascade.computed[child].vertical_align,
                container_cv.font_size.px(),
            );
            linebox_leading = linebox_leading.max(leading);
            linebox_trailing = linebox_trailing.max(trailing);
        }
        let linebox_padding_top = (linebox_leading > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.top,
                linebox_leading,
                "line-box-leading",
            )
        });
        let linebox_padding_bottom = (linebox_trailing > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.bottom,
                linebox_trailing,
                "line-box-trailing",
            )
        });
        {
            let style = &mut doc.nodes[idx].style;
            if let Some(padding) = linebox_padding_top {
                style.padding.top = padding;
            }
            if let Some(padding) = linebox_padding_bottom {
                style.padding.bottom = padding;
            }
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = if has_forced_break || has_ch_indent {
                TaffyFlexWrap::Wrap
            } else {
                TaffyFlexWrap::NoWrap
            };
            style.align_items = Some(TaffyAlignItems::BASELINE);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            // `text-align: center` の line box 全体の中央寄せは flex の
            // main-axis 配置で実現する (parley 側ではなく container 側)。
            // 他値は `None` のまま (本 task の scope 外 — `Right` 等の flex
            // 対応は将来 work)。
            style.justify_content = if cascade.computed[idx].text_align == TextAlign::Center {
                Some(TaffyAlignContent::CENTER)
            } else {
                None
            };
            style.gap = Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            };
        }
        for c in participating_children {
            let is_break = is_forced_line_break(doc, c, cascade);
            let strip_inline_padding = cascade.computed[c].display == DisplayValue::Inline
                && has_line_box_edge_aligned_descendant(doc, c, cascade);
            let child_style = &mut doc.nodes[c].style;
            if strip_inline_padding {
                // The wrapper is bridged as a block box, so its block-axis
                // padding would move a nested top/bottom-aligned descendant
                // away from the outer line edge. For this narrow edge-aligned
                // slice, remove that padding from the wrapper's block layout.
                child_style.padding.top = LengthPercentage::length(0.0);
                child_style.padding.bottom = LengthPercentage::length(0.0);
            }
            child_style.flex_grow = 0.0;
            child_style.flex_shrink = 0.0;
            child_style.flex_basis = if is_break {
                Dimension::percent(1.0)
            } else {
                Dimension::auto()
            };
            child_style.align_self = match cascade.computed[c].vertical_align {
                VerticalAlign::Top => Some(TaffyAlignSelf::FLEX_START),
                VerticalAlign::Bottom => Some(TaffyAlignSelf::FLEX_END),
                _ => None,
            };
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

/// Return whether an inline wrapper contains a descendant whose aligned
/// subtree is explicitly pinned to a line-box edge.
///
/// The minimal bridge may keep a nested inline wrapper on taffy's block path
/// when it has no inline element child. A wrapper with
/// `padding-top`/`padding-bottom` would therefore move both its text and the
/// aligned descendant, unlike the CSS line-box model. The caller uses this
/// predicate only for the focused `top`/`bottom` slice.
fn has_line_box_edge_aligned_descendant(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
) -> bool {
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(child) = stack.pop() {
        if doc.nodes[child].kind() == NodeKind::Element {
            if matches!(
                cascade.computed[child].vertical_align,
                VerticalAlign::Top | VerticalAlign::Bottom
            ) {
                return true;
            }
            stack.extend(doc.nodes[child].children.iter().copied());
        }
    }
    false
}

/// `idx` (ある [`NodeKind::Element`]) が
/// [`establish_minimal_line_boxes`] の minimal-line-box 処理の対象かどうか
/// — qualifying condition とその根拠は同関数の doc 参照。
fn qualifies_for_minimal_line_box(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
    has_autospace_candidate: bool,
) -> bool {
    // ここでは意図的に pre-bridge の `DisplayValue` を読む
    // (post-`bridge_display` の `taffy::Display` ではない — この second
    // pass が走る時点で `Block` / `Inline` / `InlineBlock` は既に全て
    // `Display::Block` に collapse 済み)。plain な `Inline` は direct text
    // children だけなら除外し、nested inline element child を持つ wrapper
    // だけを qualify する (この module の doc 参照)。
    // table 系 (`Table` / `InlineTable` / `TableRow` / `TableCell` /
    // `TableCaption`) は全 child inline の場合に限り qualify する —
    // match arm 上の注記参照。taffy dispatch 側
    // (`taffy_impl.rs` の table dispatch) は `IS_INLINE_ROOT` flag を見て
    // table engine を bypass する。
    let is_plain_inline = has_autospace_candidate
        && cascade.computed[idx].display == DisplayValue::Inline
        && cascade.computed[idx].text_autospace != TextAutospace::NoAutospace;
    if cascade.computed[idx].display == DisplayValue::Inline && !is_plain_inline {
        return false;
    }
    let has_inline_element_child = is_plain_inline
        && doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element && is_inline_element_box(cascade, child)
        });
    match cascade.computed[idx].display {
        DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::Inline => {}
        // Table-internal boxes whose children are ALL inline-level get no
        // anonymous fixup from the table engine (it only collects rows and
        // cells) — without this they stack vertically in taffy's block
        // path. A pure-inline table/row/cell/caption is exactly one
        // anonymous cell's content (css-tables-3 §2.2.1 fixup with a single
        // run), so flowing it inline here is equivalent. Boxes with any
        // cell/row (or other block-level) child keep the table path via the
        // disqualify arm below. Column groups/columns never have renderable
        // children and stay disqualified.
        DisplayValue::Table
        | DisplayValue::InlineTable
        | DisplayValue::TableRow
        | DisplayValue::TableCell
        | DisplayValue::TableCaption => {}
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
    if is_plain_inline {
        // A plain inline with a nested inline element needs a synthetic line
        // container so nested descendants do not take the block path and
        // stack vertically. Keep direct text-only inline containers on the
        // historical path; their content already participates in the parent
        // line box and existing layout behavior remains unchanged.
        has_inline_element_child && inline_level_count >= 1
    } else {
        inline_level_count >= 2
    }
}

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

/// 非有限 (`NaN` / `±Inf`) または `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]`
/// 範囲外の `font-weight` を `parley::FontWeight::new` へ渡す直前で
/// sanitize する。
///
/// # なぜここに置くか
///
/// 「非有限 / 範囲外 f32 の guard は値が実際に使われる sink 境界
/// (target context) に置く。parse-time (specified 層) にも resolve 層
/// (computed 層) にも置かない」という方針に従う。[`raikiri_style::page::cascade_page`]
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

/// `raikiri_style::property::FontStyle` (`Normal | Italic | Oblique`,
/// `#[non_exhaustive]`) → parley's `FontStyle` (re-exported from the
/// `parlance` crate: `Normal | Italic | Oblique(Option<f32>)`, CSS Fonts 4
/// §2.4 <https://www.w3.org/TR/css-fonts-4/#font-style-prop>).
///
/// `Oblique` has no `<angle>` payload on the raikiri-style side (bare
/// keyword only — see `raikiri_style::property::FontStyle` doc's "Scope
/// carving" section), so it maps to parley's `Oblique(None)`, which per
/// parley's own doc uses the engine-specific default oblique angle. The
/// wildcard arm exists purely for `StyleFontStyle`'s `#[non_exhaustive]`
/// forward-compat contract (a downstream match must tolerate variants
/// added to the source enum later, e.g. a future `<angle>`-bearing
/// oblique) and is unreachable with the variant set that exists today.
fn font_style_to_parley(v: StyleFontStyle) -> FontStyle {
    match v {
        StyleFontStyle::Normal => FontStyle::Normal,
        StyleFontStyle::Italic => FontStyle::Italic,
        StyleFontStyle::Oblique => FontStyle::Oblique(None),
        // cov:ignore: unreachable while StyleFontStyle is
        // Normal|Italic|Oblique only; required for its #[non_exhaustive]
        // contract (see doc above).
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
pub(crate) fn sanitize_line_height(
    v: ComputedLineHeight,
    diag: &mut Vec<LayoutWarn>,
) -> ComputedLineHeight {
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
/// - `text_align` — 本関数の末尾の `layout.align(...)` は意図的に
///   `Alignment::Start` に固定し、`cv.text_align` を読まない。
///   正しい幅基準の align は taffy 後の [`realign_text_after_layout`] が担う
///   (確定した containing block 幅で `break_all_lines` + `align` し直す)。
///   本関数で素朴に enum mapping してしまうと、使える幅が `max_advance`
///   (= 通常 `page_box.width`) のみのため、狭い containing block 内の
///   `Center` / `Right` / `End` / `Justify` がページ幅基準にズレる。
///   `Start` は幅に依存しないためここでも正しい。
///   加えて `parley::Alignment::Start` / `End` は layout 内の bidi 解析結果から
///   physical 方向を解決するため、`direction` を配線せずに `text_align`
///   だけ配線しても `Start`/`End` は content-inferred direction での解決に
///   留まる — この 2 つは独立した gap ではなく 1 セットであり、`direction`
///   の配線口は parley public API に存在しない (上記 `direction` bullet 参照)。
///   flex 化された line box (`IS_INLINE_ROOT`) の中央寄せは parley 側ではなく
///   container 側の `justify_content` が担う
///   ([`establish_minimal_line_boxes`] doc 参照)。
///
/// # 失敗しない
///
/// 以前は `Result<(), LayoutError>` を返していた。唯一の `Err` 経路は
/// `cv.font_size` が specified 層の `Length` で `Px` 以外だった場合の
/// `LayoutError::Internal` だったが、`font_size` が [`ComputedLength`] (px) に
/// なって match 自体が消えたため到達不能になった。`pub(crate)` なので戻り値の
/// narrowing は crate 内で完結する (外部影響 0)。
///
/// Preserve the previous per-text-node expansion for inline contexts whose
/// shared line cursor and wrap positions are not available to pre-shaping.
/// Also returns the byte length each tab was replaced by (in source order) so
/// offsets into `text` can be remapped.
fn expand_tabs_locally(
    text: &str,
    tab_size: ComputedTabSize,
    space_advance: f32,
) -> (String, Vec<usize>) {
    let stop = match tab_size {
        ComputedTabSize::Number(number) if number > 0.0 && number.is_finite() => number,
        ComputedTabSize::Length(length)
            if length.px().is_finite() && length.px() >= 0.0 && space_advance > 0.0 =>
        {
            length.px() / space_advance
        }
        _ => 0.0,
    };
    let mut output = String::with_capacity(text.len());
    let mut tab_lens = Vec::new();
    if stop <= 0.0 {
        output.extend(text.chars().filter(|&character| character != '\t'));
        tab_lens.resize(text.matches('\t').count(), 0);
        return (output, tab_lens);
    }
    let mut column = 0.0_f32;
    for character in text.chars() {
        match character {
            '\n' => {
                column = 0.0;
                output.push(character);
            }
            '\t' => {
                let next = ((column / stop).floor() + 1.0) * stop;
                let count = (next.round() - column.round()).max(0.0) as usize;
                column = next;
                output.extend(std::iter::repeat_n(' ', count));
                tab_lens.push(count);
            }
            _ => {
                column += 1.0;
                output.push(character);
            }
        }
    }
    (output, tab_lens)
}

/// Move inline-box offsets computed on `original` onto the text produced by a
/// rewrite that replaced its tabs (in source order) by `tab_lens` bytes and
/// kept every other character. Autospace boundaries are detected before tabs
/// are rewritten so a tab still separates its neighbors even when it expands
/// to nothing.
fn remap_boxes_through_tab_rewrite(original: &str, tab_lens: &[usize], boxes: &mut [InlineBox]) {
    let tabs: Vec<usize> = original.match_indices('\t').map(|(at, _)| at).collect();
    debug_assert_eq!(tabs.len(), tab_lens.len());
    for inline_box in boxes {
        let shift: isize = tabs
            .iter()
            .zip(tab_lens)
            .take_while(|&(&at, _)| at < inline_box.index)
            .map(|(_, &len)| len as isize - 1)
            .sum();
        inline_box.index = inline_box.index.saturating_add_signed(shift);
    }
}

const LINE_BREAK_NBSP_INLINE_BOX_ID_BASE: u64 = 1 << 62;
const LINE_BREAK_NBSP_INLINE_BOX_ID_LIMIT: u64 = 1 << 63;

/// Replace U+00A0 in Parley's input with a nonpainting advance box.
///
/// Existing inline-box offsets are based on the source string and must move
/// left by two UTF-8 bytes for every preceding NBSP that is removed.
fn replace_nbsp_with_inline_boxes(
    text: &str,
    advance: f32,
    existing_boxes: &[InlineBox],
) -> Option<(String, Vec<InlineBox>)> {
    if !advance.is_finite() || advance <= 0.0 {
        return None;
    }

    let nbsp_offsets: Vec<usize> = text
        .match_indices('\u{00A0}')
        .map(|(offset, _)| offset)
        .collect();
    if nbsp_offsets.is_empty() {
        return Some((text.to_owned(), existing_boxes.to_vec()));
    }

    let id_count = u64::try_from(nbsp_offsets.len()).ok()?;
    let last_id = LINE_BREAK_NBSP_INLINE_BOX_ID_BASE.checked_add(id_count.checked_sub(1)?)?;
    if last_id >= LINE_BREAK_NBSP_INLINE_BOX_ID_LIMIT
        || existing_boxes.iter().any(|inline_box| {
            (LINE_BREAK_NBSP_INLINE_BOX_ID_BASE..LINE_BREAK_NBSP_INLINE_BOX_ID_LIMIT)
                .contains(&inline_box.id)
        })
    {
        return None;
    }

    let mut output = String::with_capacity(text.len());
    let mut adapter_boxes = Vec::with_capacity(nbsp_offsets.len());
    for (source_index, character) in text.char_indices() {
        if character == '\u{00A0}' {
            let ordinal = u64::try_from(adapter_boxes.len()).ok()?;
            adapter_boxes.push(InlineBox {
                id: LINE_BREAK_NBSP_INLINE_BOX_ID_BASE.checked_add(ordinal)?,
                kind: InlineBoxKind::InFlow,
                index: output.len(),
                width: advance,
                height: 0.0,
            });
        } else {
            debug_assert!(source_index <= text.len());
            output.push(character);
        }
    }

    let mut ordered_boxes = Vec::with_capacity(existing_boxes.len() + adapter_boxes.len());
    for (order, inline_box) in existing_boxes.iter().enumerate() {
        let source_index = inline_box.index;
        if source_index > text.len() || !text.is_char_boundary(source_index) {
            return None;
        }
        let removed_bytes = nbsp_offsets
            .partition_point(|&offset| offset < source_index)
            .checked_mul('\u{00A0}'.len_utf8())?;
        let output_index = source_index.checked_sub(removed_bytes)?;
        let mut inline_box = inline_box.clone();
        inline_box.index = output_index;
        // At a shared output index, the original source boundary determines
        // whether a box was before, between, or after removed NBSP characters.
        ordered_boxes.push((output_index, source_index, 0_u8, order, inline_box));
    }
    for (order, (source_index, inline_box)) in
        nbsp_offsets.iter().copied().zip(adapter_boxes).enumerate()
    {
        let output_index = inline_box.index;
        ordered_boxes.push((output_index, source_index, 1_u8, order, inline_box));
    }
    ordered_boxes.sort_by_key(|(output_index, source_index, kind, order, _)| {
        (*output_index, *source_index, *kind, *order)
    });
    let boxes = ordered_boxes
        .into_iter()
        .map(|(_, _, _, _, inline_box)| inline_box)
        .collect();
    Some((output, boxes))
}

/// Resolve `tab-size` to an absolute stop interval in CSS pixels.
fn tab_stop_advance(tab_size: ComputedTabSize, block_space_advance: f32) -> f32 {
    match tab_size {
        ComputedTabSize::Number(number)
            if number.is_finite() && number >= 0.0 && block_space_advance.is_finite() =>
        {
            (number * block_space_advance).clamp(0.0, MAX_TAFFY_MAGNITUDE)
        }
        ComputedTabSize::Length(length) if length.px().is_finite() && length.px() >= 0.0 => {
            length.px().min(MAX_TAFFY_MAGNITUDE)
        }
        _ => 0.0,
    }
}

/// Byte ranges of tab replacement spaces and the word spacing each needs.
type TabSpacingRanges = Vec<(std::ops::Range<usize>, f32)>;

/// Replace each tab by an invisible U+0020 with ranged WordSpacing so its
/// advance exactly reaches the next stop. Keeping a normal glyph run preserves
/// the text's font metrics and line height; the ranged spacing supplies the
/// block-container-derived physical width when the inline font differs.
///
/// Also returns the byte length each tab was replaced by (in source order).
pub(crate) fn replace_tabs_with_styled_spaces(
    text: &str,
    interval: f32,
    space_base_advance: f32,
    mut measure_segment: impl FnMut(&str) -> f32,
) -> (String, TabSpacingRanges, Vec<usize>) {
    if !text.contains('\t') {
        return (text.to_owned(), Vec::new(), Vec::new());
    }
    let mut tab_lens = Vec::new();
    let mut output = String::with_capacity(text.len());
    let mut spacing_ranges = Vec::new();
    let mut segment = String::new();
    let mut cursor = 0.0_f32;
    let mut flush_segment = |segment: &mut String, output: &mut String, cursor: &mut f32| {
        if segment.is_empty() {
            return;
        }
        let measured = measure_segment(segment);
        if measured.is_finite() && measured > 0.0 {
            *cursor = (*cursor + measured).min(MAX_TAFFY_MAGNITUDE);
        }
        output.push_str(segment);
        segment.clear();
    };
    for character in text.chars() {
        match character {
            '\t' => {
                flush_segment(&mut segment, &mut output, &mut cursor);
                let before = output.len();
                if interval > 0.0 && interval.is_finite() {
                    let next = ((cursor / interval).floor() + 1.0) * interval;
                    let gap = (next - cursor).max(0.0);
                    if gap > 0.0 {
                        let start = output.len();
                        output.push(' ');
                        let word_spacing = (gap - space_base_advance)
                            .clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE);
                        spacing_ranges.push((start..output.len(), word_spacing));
                    }
                    cursor = next.min(MAX_TAFFY_MAGNITUDE);
                }
                tab_lens.push(output.len() - before);
            }
            '\n' => {
                flush_segment(&mut segment, &mut output, &mut cursor);
                output.push(character);
                cursor = 0.0;
            }
            _ => segment.push(character),
        }
    }
    flush_segment(&mut segment, &mut output, &mut cursor);
    (output, spacing_ranges, tab_lens)
}

/// Measure one probe glyph/character advance (px) with Parley.
///
/// The caller chooses the sample because tabs use U+0020 while `ch` uses the
/// U+0030 zero glyph. Non-finite, zero, or implausibly large results fall back
/// to `font_size * 0.5` as a fail-safe.
///
/// [`Layout::width`]: parley::Layout::width
fn probe_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        sample,
        family_str,
        font_size_px,
        font_weight,
        font_style,
        false,
    )
}

fn probe_ch_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        "0",
        family_str,
        font_size_px,
        font_weight,
        font_style,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn probe_text_advance_inner(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    allow_zero: bool,
) -> f32 {
    let mut warnings: Vec<LayoutWarn> = Vec::new();
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(font_weight, &mut warnings);
    let family = FontFamily::from(family_str);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(family));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    let w = layout.width();
    let has_glyph_run = allow_zero
        && layout
            .lines()
            .flat_map(|line| line.items())
            .any(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
                _ => false, // cov:ignore: this helper shapes plain text and cannot create non-glyph items.
            });
    if w.is_finite() && (w > 0.0 || has_glyph_run) && w <= size * 4.0 {
        w
    } else {
        size * 0.5
    }
}

#[derive(Clone, Copy)]
struct TextProbeStyle<'a> {
    family_str: &'a str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    letter_spacing: f32,
    word_spacing: f32,
}

fn probe_text_full_width(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    style: TextProbeStyle<'_>,
) -> f32 {
    let mut warnings = Vec::new();
    let size = sanitize_finite(
        style.font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(style.font_weight, &mut warnings);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(
        style.family_str,
    )));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
        style.font_style,
    )));
    builder.push_default(StyleProperty::LetterSpacing(style.letter_spacing));
    builder.push_default(StyleProperty::WordSpacing(style.word_spacing));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    layout.full_width()
}

/// Return whether the requested face maps both the space and zero glyphs.
///
/// CSS's first-available `ch` face must be usable for the space and must also
/// provide U+0030, whose advance supplies the metric. A mapped zero-advance
/// glyph is valid; cmap presence is the only coverage check here.
fn family_candidate_has_ch_glyphs(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Return whether any face in a generic family maps both required glyphs.
fn generic_family_has_ch_glyphs(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Probe U+0030 using the first remaining family that can supply the glyph.
///
/// Named faces are checked through Fontique cmap metadata. An unregistered
/// name is skipped so a later available family can win; when a generic family
/// is reached, Parley receives the remaining authored list so its fallback
/// resolver preserves generic and later-family order. A valid zero-advance
/// mapped glyph remains accepted by `probe_ch_zero_advance`.
fn probe_ch_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        if raw_candidate.is_empty() {
            // cov:ignore: candidates were already filtered for emptiness.
            continue;
        }
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            None
        } else {
            let lower_name = name.to_ascii_lowercase();
            parley::fontique::GenericFamily::parse(&lower_name)
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        let has_glyphs =
            registered && family_candidate_has_ch_glyphs(fonts, name, font_weight, font_style);
        if let Some(generic) = generic {
            // An unresolved named face may represent an @font-face source
            // that the WPT loader could not activate (for example a missing
            // or malformed web-font resource). Do not silently replace that unavailable
            // face with a generic metric; the style-layer fallback is the
            // deterministic 0.5em result for this no-face case.
            if saw_unregistered_named
                || !generic_family_has_ch_glyphs(fonts, generic, font_weight, font_style)
            {
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if has_glyphs {
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    // No remaining family advertises U+0030. Do not shape the rejected list
    // again: Parley may emit a .notdef run whose advance would masquerade as
    // a valid zero-width glyph. Match the style-layer fallback instead.
    let mut warnings = Vec::new();
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    ) * 0.5
}

/// Return whether a named face maps U+6C34, the character used by CSS `ic`.
fn family_candidate_has_ic_glyph(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_glyph = false;
    query.matches_with(|font| {
        has_glyph = font
            .charmap()
            .is_some_and(|charmap| charmap.map(0x6C34_u32).is_some_and(|glyph| glyph != 0));
        if has_glyph {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_glyph
}

/// Return whether a generic family has any face mapping U+6C34.
fn generic_family_has_ic_glyph(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        // cov:ignore: generic-family availability is platform-dependent and may be empty.
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_glyph = false;
    query.matches_with(|font| {
        has_glyph = font
            .charmap()
            .is_some_and(|charmap| charmap.map(0x6C34_u32).is_some_and(|glyph| glyph != 0));
        if has_glyph {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_glyph
}

/// Measure U+6C34 using the first available family that maps it.
///
/// CSS `ic` uses the ideographic advance from a mapped face. Do not accept a
/// `.notdef` advance when a family lacks U+6C34, and preserve a genuine zero
/// advance. If no candidate maps the character, use the specified 1em fallback.
///
/// The computed family list currently does not retain `@font-face`
/// `unicode-range` provenance for this metric. Such alias-specific filtering
/// remains a known limitation; ordinary registered-family and generic fallback
/// selection is still checked against cmap metadata here.
fn probe_ic_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            // Quoted generic-looking names are ordinary family names.
            None
        } else {
            parley::fontique::GenericFamily::parse(&name.to_ascii_lowercase())
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        if let Some(generic) = generic {
            if saw_unregistered_named
                || !generic_family_has_ic_glyph(fonts, generic, font_weight, font_style)
            {
                // An earlier unavailable family prevents this generic fallback.
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ic_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if registered && family_candidate_has_ic_glyph(fonts, name, font_weight, font_style) {
            return probe_ic_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut Vec::new(),
    )
}

fn probe_ic_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut Vec::new(),
    );
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut builder = layout_cx.ranged_builder(fonts, "水", 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family_str)));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build("水");
    layout.break_all_lines(None);
    let has_glyph = layout
        .lines()
        .flat_map(|line| line.items())
        .any(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
            // cov:ignore: a plain single-character metric layout is expected to contain only glyph runs.
            _ => false,
        });
    let width = layout.full_width();
    // Keep the same 4em malformed-font guard used by the existing `ch`
    // metric probe while accepting zero as a valid mapped advance.
    if has_glyph && width.is_finite() && width >= 0.0 && width <= size * 4.0 {
        width
    } else {
        // cov:ignore: malformed or missing-glyph metric fallback is defensive.
        size
    }
}

/// Measure the `ch` advance needed by a computed `text-indent` value.
///
/// The cache key mirrors the text shaping face selection used by
/// [`probe_text_advance`], and the caller supplies the same font context that
/// shaped the document's text. The caller supplies the authored factor only
/// for `ch` values; this function returns the clamped used px value.
fn measured_ch_length_px(
    factor: f32,
    source: Option<&raikiri_style::ChFontKey>,
    cv: &ComputedValues,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
) -> f32 {
    let family_atoms = source.map(|key| &key.family).unwrap_or(&cv.font_family);
    let family = family_atoms
        .iter()
        .map(|a| a.0.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let size = source.map_or(cv.font_size, |key| key.size);
    let weight = source.map_or(cv.font_weight, |key| key.weight);
    let font_style = source.map_or(cv.font_style, |key| key.style);
    let style = match font_style {
        StyleFontStyle::Normal => 0,
        StyleFontStyle::Italic => 1,
        StyleFontStyle::Oblique => 2,
        _ => 0,
    };
    let key = (family.clone(), size.px().to_bits(), weight.to_bits(), style);
    let advance = *probes.entry(key).or_insert_with(|| {
        probe_ch_text_advance(fonts, layout_cx, &family, size.px(), weight, font_style)
    });
    let used = factor * advance;
    if used.is_nan() {
        0.0
    } else {
        used.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

fn measured_text_indent_px(
    cv: &ComputedValues,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
) -> Option<f32> {
    let factor = cv.text_indent_ch_factor?;
    Some(measured_ch_length_px(
        factor,
        cv.text_indent_ch_font.as_ref(),
        cv,
        fonts,
        layout_cx,
        probes,
    ))
}

pub(crate) fn bounded_text_indent_amount(
    value: ComputedLengthPercentage,
    containing_width: f32,
    measured: Option<f32>,
) -> f32 {
    let raw = match value {
        ComputedLengthPercentage::Px(px) => measured.unwrap_or(px),
        ComputedLengthPercentage::Percent(percent) => containing_width * percent / 100.0,
    };
    if raw.is_nan() {
        0.0
    } else {
        raw.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

/// [`ComputedLength`]: raikiri_style::ComputedLength
/// [`ComputedLineHeight`]: raikiri_style::ComputedLineHeight
/// CSS white-space characters (CSS Text 3 §4.1.1): space, tab, LF, FF, CR.
fn is_css_white_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// Zero-width space (U+200B): a segment break adjacent to it is removed
/// without leaving a space (CSS Text 3 §4.1.2 rule 1).
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// Hangul blocks for the CSS Text 3 §4.1.2 Hangul carve-out (a break with
/// Hangul on either side is NOT removed even between wide characters):
/// Jamo, Compatibility Jamo, Extended-A/B, Syllables. Ranges are stable
/// since Unicode 2.0 and need no data tables.
fn is_hangul_for_break(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'
            | '\u{3130}'..='\u{318F}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{D7B0}'..='\u{D7FF}'
    )
}

/// East Asian Width map for segment-break decisions (CSS Text 3 §4.1.2
/// rule 2: removal when both sides are Fullwidth/Wide/Halfwidth).
/// Loaded once per process from ICU compiled data (same provider the
/// parley dependency already links; no new data pulled in).
fn east_asian_width_map()
-> icu_properties::CodePointMapDataBorrowed<'static, icu_properties::props::EastAsianWidth> {
    icu_properties::CodePointMapDataBorrowed::<icu_properties::props::EastAsianWidth>::new()
}

/// Whether `c` is a default-ignorable code point for segment-break
/// neighbor selection, excluding U+200B whose explicit rule is handled by
/// the caller.
fn is_segment_break_ignorable(c: char) -> bool {
    // U+200B has an explicit segment-break rule and must remain visible to
    // the caller; other default-ignorable code points (variation selectors,
    // soft hyphen, LRM, etc.) do not participate in the EAW neighbor test.
    c != ZERO_WIDTH_SPACE
        && icu_properties::CodePointSetData::new::<
            icu_properties::props::DefaultIgnorableCodePoint,
        >()
        .contains(c)
}

/// Whether `c` counts as wide for break removal: East Asian Width
/// Fullwidth/Wide/Halfwidth (Ambiguous excluded) and not Hangul (which
/// the spec carves out even when wide, e.g. Hangul syllables).
fn is_wide_for_break(c: char) -> bool {
    use icu_properties::props::EastAsianWidth;
    let ea = east_asian_width_map().get(c);
    (ea == EastAsianWidth::Fullwidth
        || ea == EastAsianWidth::Wide
        || ea == EastAsianWidth::Halfwidth)
        && !is_hangul_for_break(c)
}

/// Deepest edge text char inside element `elem` (`dir` -1: last,
/// +1: first), skipping `display:none` subtrees and empty text.
/// Returns `None` when the element holds no text (e.g. empty inline or
/// image): callers treat that as narrow, never as a boundary.
fn deep_edge_char(doc: &Document, cascade: &CascadeResult, elem: usize, dir: i8) -> Option<char> {
    let kids = &doc.nodes[elem].children;
    let range: Box<dyn Iterator<Item = usize>> = if dir < 0 {
        Box::new((0..kids.len()).rev())
    } else {
        Box::new(0..kids.len())
    };
    for i in range {
        let c = kids[i];
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => {
                let t = text_of(doc, c)?;
                if t.is_empty() {
                    continue;
                }
                return if dir < 0 {
                    t.chars().next_back()
                } else {
                    t.chars().next()
                };
            }
            NodeKind::Element => {
                if cascade.computed[c].display == DisplayValue::None {
                    continue;
                }
                if let Some(ch) = deep_edge_char(doc, cascade, c, dir) {
                    return Some(ch);
                }
            }
            _ => continue,
        }
    }
    None
}

/// Directly neighboring char of text node `idx` (`dir` -1: char before,
/// +1: char after) in reading order: in-sibling text edge (including
/// collapsible spaces — adjacency to a space keeps it), else the edge
/// text of a neighboring element (deep), else ascending through inline
/// ancestors like [`has_inline_adjacent`]. `None` at a block boundary or
/// when no text is found (treated as narrow, never as a break).
fn edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let p = parent_of[node]?;
        let kids = &doc.nodes[p].children;
        let pos = kids.iter().position(|&c| c == node)?;
        let mut i = pos as isize + dir as isize;
        while i >= 0 && (i as usize) < kids.len() {
            let sib = kids[i as usize];
            i += dir as isize;
            if !doc.nodes[sib].is_in_document() {
                continue;
            }
            match doc.nodes[sib].kind() {
                NodeKind::Text => {
                    let t = text_of(doc, sib)?;
                    if t.is_empty() {
                        continue;
                    }
                    return if dir < 0 {
                        t.chars().next_back()
                    } else {
                        t.chars().next()
                    };
                }
                NodeKind::Element => {
                    if cascade.computed[sib].display == DisplayValue::None {
                        continue;
                    }
                    if let Some(ch) = deep_edge_char(doc, cascade, sib, dir) {
                        return Some(ch);
                    }
                    continue;
                }
                _ => continue,
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return None;
    }
}

/// Check one side of a text node for a neighboring whitespace-only node that
/// contains a segment break. Unlike `whitespace_run_has_break`, this keeps
/// the direction so a leading space prefix is not associated with a later,
/// unrelated break in the same inline sequence.
fn whitespace_edge_has_break(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let Some(parent) = parent_of[idx] else {
        return false;
    };
    let Some(pos) = doc.nodes[parent].children.iter().position(|&c| c == idx) else {
        return false;
    };
    let mut i = pos as isize + dir as isize;
    while i >= 0 && (i as usize) < doc.nodes[parent].children.len() {
        let sibling = doc.nodes[parent].children[i as usize];
        i += dir as isize;
        if !doc.nodes[sibling].is_in_document() {
            continue;
        }
        if doc.nodes[sibling].kind() == NodeKind::Element
            && cascade.computed[sibling].display == DisplayValue::None
        {
            continue;
        }
        if doc.nodes[sibling].kind() != NodeKind::Text {
            break;
        }
        let Some(sibling_text) = text_of(doc, sibling) else {
            break;
        };
        if !sibling_text.chars().all(is_css_white_space) {
            break;
        }
        if sibling_text.contains(['\n', '\r']) {
            return true;
        }
    }
    false
}

/// Remove whitespace runs whose segment break is removable because of the
/// significant characters on both sides. The ordinary collapse pass still
/// handles all other runs; this narrow pre-pass only prevents it from
/// turning a removable CJK break into a space after the parser combines the
/// surrounding indentation and character into one text node.
fn remove_removable_segment_break_runs(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    s: &str,
) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < chars.len() {
        if !is_css_white_space(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_css_white_space(chars[i]) {
            i += 1;
        }
        let end = i;
        let has_break = chars[start..end]
            .iter()
            .any(|&ch| matches!(ch, '\n' | '\r'))
            || (start == 0 && whitespace_edge_has_break(doc, cascade, parent_of, idx, -1))
            || (end == chars.len() && whitespace_edge_has_break(doc, cascade, parent_of, idx, 1));
        if !has_break {
            out.extend(chars[start..end].iter().copied());
            continue;
        }
        let significant =
            |ch: &&char| !is_css_white_space(**ch) && !is_segment_break_ignorable(**ch);
        let before = chars[..start]
            .iter()
            .rev()
            .find(significant)
            .copied()
            .or_else(|| edge_non_whitespace_char(doc, cascade, parent_of, idx, -1));
        let after = chars[end..]
            .iter()
            .find(significant)
            .copied()
            .or_else(|| edge_non_whitespace_char(doc, cascade, parent_of, idx, 1));
        let removable = before == Some(ZERO_WIDTH_SPACE)
            || after == Some(ZERO_WIDTH_SPACE)
            || matches!((before, after), (Some(a), Some(b)) if is_wide_for_break(a) && is_wide_for_break(b));
        if !removable {
            out.extend(chars[start..end].iter().copied());
        }
    }
    out
}

/// Whether the contiguous text-node whitespace run around `idx` contains
/// a segment break. This handles the common HTML-tokenizer shape where a
/// run's spaces and character-reference LF tokens become separate siblings.
/// A mixed text node is intentionally not crossed: its own collapse pass has
/// already seen the complete text and must remain the owner of that run.
fn whitespace_run_has_break(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    text: &str,
) -> bool {
    if text.contains(['\n', '\r']) {
        return true;
    }
    let Some(parent) = parent_of[idx] else {
        return false;
    };
    let Some(pos) = doc.nodes[parent].children.iter().position(|&c| c == idx) else {
        return false;
    };
    for dir in [-1isize, 1] {
        let mut i = pos as isize + dir;
        while i >= 0 && (i as usize) < doc.nodes[parent].children.len() {
            let sibling = doc.nodes[parent].children[i as usize];
            i += dir;
            if !doc.nodes[sibling].is_in_document() {
                continue;
            }
            if doc.nodes[sibling].kind() == NodeKind::Element
                && cascade.computed[sibling].display == DisplayValue::None
            {
                continue;
            }
            if doc.nodes[sibling].kind() != NodeKind::Text {
                break;
            }
            let Some(sibling_text) = text_of(doc, sibling) else {
                break;
            };
            if !sibling_text.chars().all(is_css_white_space) {
                break;
            }
            if sibling_text.contains(['\n', '\r']) {
                return true;
            }
        }
    }
    false
}

/// Find the nearest non-CSS-whitespace character at an element edge.
///
/// This is deliberately different from `deep_edge_char`: segment-break
/// transformation has to look through the whole collapsible run, not merely
/// at the first LF/space text node in that run. The HTML tokenizer commonly
/// creates one text node per character reference, so a run such as
/// `一些\n\n\n中文` is represented by several adjacent whitespace nodes.
fn deep_edge_non_whitespace_char(
    doc: &Document,
    cascade: &CascadeResult,
    elem: usize,
    dir: i8,
) -> Option<char> {
    let kids = &doc.nodes[elem].children;
    let range: Box<dyn Iterator<Item = usize>> = if dir < 0 {
        Box::new((0..kids.len()).rev())
    } else {
        Box::new(0..kids.len())
    };
    for i in range {
        let c = kids[i];
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => {
                let Some(t) = text_of(doc, c) else {
                    continue;
                };
                let edge = if dir < 0 {
                    t.chars()
                        .rev()
                        .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                } else {
                    t.chars()
                        .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                };
                if edge.is_some() {
                    return edge;
                }
            }
            NodeKind::Element => {
                if cascade.computed[c].display == DisplayValue::None {
                    continue;
                }
                if let Some(ch) = deep_edge_non_whitespace_char(doc, cascade, c, dir) {
                    return Some(ch);
                }
            }
            _ => continue,
        }
    }
    None
}

/// Directly neighboring non-whitespace character of a text node in reading
/// order. CSS-whitespace-only text nodes are skipped so callers can evaluate
/// one segment-break run even when the parser split it into many nodes.
fn edge_non_whitespace_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let p = parent_of[node]?;
        let kids = &doc.nodes[p].children;
        let pos = kids.iter().position(|&c| c == node)?;
        let mut i = pos as isize + dir as isize;
        while i >= 0 && (i as usize) < kids.len() {
            let sib = kids[i as usize];
            i += dir as isize;
            if !doc.nodes[sib].is_in_document() {
                continue;
            }
            match doc.nodes[sib].kind() {
                NodeKind::Text => {
                    let Some(t) = text_of(doc, sib) else {
                        continue;
                    };
                    let edge = if dir < 0 {
                        t.chars()
                            .rev()
                            .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                    } else {
                        t.chars()
                            .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                    };
                    if edge.is_some() {
                        return edge;
                    }
                }
                NodeKind::Element => {
                    if cascade.computed[sib].display == DisplayValue::None {
                        continue;
                    }
                    if let Some(ch) = deep_edge_non_whitespace_char(doc, cascade, sib, dir) {
                        return Some(ch);
                    }
                }
                _ => continue,
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return None;
    }
}

/// Whether `idx` generates an inline-level box for white-space trimming.
///
/// Text nodes are inline-level; elements follow their computed display
/// (`inline`, `inline-block`, `inline-table`). `contents` is treated as
/// inline (conservative: a wrong keep preserves old behavior, a wrong drop
/// would regress). `<br>` counts as a block boundary (leading/trailing
/// spaces around a forced break collapse).
fn is_inline_for_trim(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    if doc.nodes[idx].kind() == NodeKind::Text {
        return true;
    }
    // Absolutely/fixed-positioned boxes are out of the inline flow and cannot
    // keep a collapsible space at the boundary of the following text.
    if matches!(
        cascade.computed[idx].position,
        PositionValue::Absolute | PositionValue::Fixed
    ) {
        return false;
    }
    if doc.nodes[idx].tag_name() == Some("br") {
        return false;
    }
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Whether the element's own box is inline-level (strict: `contents` and
/// `<br>` excluded — used for ancestor ascent, where a `contents` wrapper
/// must not stop the search but a real inline box does not continue it...
/// actually ascent continues through ALL inline ancestors; this helper
/// reports whether `idx` (an element) generates an inline-level box,
/// `<br>` included as inline for ascent since content inside... `<br>` has
/// no children, so it never matters here).
pub(crate) fn is_inline_element_box(cascade: &CascadeResult, idx: usize) -> bool {
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Nearest significant sibling of the node holding text `idx` in `dir`
/// (-1: preceding, +1: following), skipping `display:none` boxes and
/// whitespace-only text nodes (both generate nothing trimmable-against).
/// Returns the sibling id, or the ancestor to ascend to when the parent's
/// edge is reached without finding one (handled by the caller via
/// `inline_beyond_block_edge`).
fn significant_sibling(
    doc: &Document,
    cascade: &CascadeResult,
    parent: usize,
    pos: usize,
    dir: i8,
) -> Option<usize> {
    let kids = &doc.nodes[parent].children;
    let mut i = pos as isize + dir as isize;
    while i >= 0 && (i as usize) < kids.len() {
        let sib = kids[i as usize];
        i += dir as isize;
        if !doc.nodes[sib].is_in_document() {
            continue;
        }
        if doc.nodes[sib].kind() == NodeKind::Element
            && (cascade.computed[sib].display == DisplayValue::None
                || matches!(
                    cascade.computed[sib].position,
                    PositionValue::Absolute | PositionValue::Fixed
                ))
        {
            continue;
        }
        if doc.nodes[sib].kind() == NodeKind::Text
            && text_of(doc, sib).is_some_and(|t| t.chars().all(is_css_white_space))
        {
            continue;
        }
        return Some(sib);
    }
    None
}

/// Whether inline-level content precedes/follows text node `idx`
/// (`dir` -1/+1) for collapsible-space trimming (CSS Text 3 §4.1.2 phases
/// II–III): spaces at a block boundary are removed; between inlines kept.
/// Ascends through inline ancestors so `x<span> a</span>` keeps its space.
fn has_inline_adjacent(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let mut node = idx;
    loop {
        let p = match parent_of[node] {
            Some(p) => p,
            None => return false,
        };
        let kids = &doc.nodes[p].children;
        let pos = match kids.iter().position(|&c| c == node) {
            Some(pos) => pos,
            None => return false,
        };
        if let Some(sib) = significant_sibling(doc, cascade, p, pos, dir) {
            return is_inline_for_trim(doc, cascade, sib);
        }
        // Parent edge: ascend iff the parent itself is inline-level.
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return false;
    }
}

/// Find the nearest in-flow inline character on one side of a text node.
///
/// Text nodes are shaped independently, but autospace applies across ordinary
/// inline-element boundaries. Whitespace, block-level boxes, atomic inline
/// boxes, and isolation boundaries stop the search rather than being skipped;
/// default-ignorable code points such as variation selectors are looked past.
pub(crate) fn autospace_adjacent_edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let parent = parent_of[node]?;
        let children = &doc.nodes[parent].children;
        let position = children.iter().position(|&child| child == node)?;
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(children[..position].iter().rev())
        } else {
            Box::new(children[position + 1..].iter())
        };
        for &sibling in siblings {
            match boundary_inline_edge(doc, cascade, sibling, dir, is_segment_break_ignorable) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return None,
                InlineEdge::Char(character) => return Some(character),
            }
        }
        if doc.nodes[parent].kind() == NodeKind::Element
            && is_inline_element_box(cascade, parent)
            && !is_shaping_isolation_boundary(doc, parent)
            && !boundary_shaping_box_breaks(cascade, parent)
        {
            node = parent;
            continue;
        }
        return None;
    }
}

/// Whether a character belongs to a script whose joining behavior must
/// continue across inline box boundaries. The CSS Text boundary-shaping cases covered
/// here exercise Arabic, N'Ko, and Mongolian; a zero-width joiner at an
/// inline boundary lets Parley retain the same joining context when the DOM
/// stores each styled text node in a separate layout.
fn is_boundary_shaping_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0600..=0x06ff
            | 0x0750..=0x077f
            | 0x07c0..=0x07ff
            | 0x08a0..=0x08ff
            | 0x1800..=0x18af
            | 0xfb50..=0xfdff
            | 0xfe70..=0xfeff
    )
}

/// Add the zero-width joiners needed to preserve joining-script context at
/// inline box boundaries while each DOM text node is shaped independently.
///
/// Only the character on each edge matters: a space, an authored ZWNJ, or a
/// non-joining script at that edge already ends the joining context, so no
/// joiner is added there even when the rest of the run is joining script.
fn add_boundary_shaping_joiners(mut text: String, before: bool, after: bool) -> String {
    if after
        && text
            .chars()
            .next_back()
            .is_some_and(is_boundary_shaping_char)
    {
        text.push('\u{200d}');
    }
    if before && text.chars().next().is_some_and(is_boundary_shaping_char) {
        text.insert(0, '\u{200d}');
    }
    text
}

/// Whether an element creates a bidi isolation boundary for shaping.
///
/// The current style bridge does not expose `unicode-bidi` in computed values,
/// so cover the HTML isolation forms used by the WPT boundary-shaping tests:
/// `<bdi>` and an inline element with `dir="auto"`.
fn is_shaping_isolation_boundary(doc: &Document, node_id: usize) -> bool {
    doc.nodes[node_id].tag_name() == Some("bdi")
        || doc.nodes[node_id]
            .attribute("dir")
            .is_some_and(|value| value.eq_ignore_ascii_case("auto"))
}

/// Whether a boundary box prevents joining across its inline content.
///
/// Nonzero physical inline-edge margins, padding, and borders are shaping
/// boundaries; outline and text decoration are not. The current style bridge
/// normalizes writing mode to horizontal-tb, so the inline edges are left and
/// right. Atomic inline-level boxes also cannot share a shaping context with
/// their siblings.
fn boundary_shaping_box_breaks(cascade: &CascadeResult, node_id: usize) -> bool {
    let cv = &cascade.computed[node_id];
    if !matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents) {
        return true;
    }
    let nonzero_length_percentage = |value: ComputedLengthPercentage| match value {
        ComputedLengthPercentage::Px(px) | ComputedLengthPercentage::Percent(px) => {
            px.abs() > f32::EPSILON
        }
    };
    let nonzero_length_percentage_or_auto = |value: ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() > f32::EPSILON
        }
        ComputedLengthPercentageOrAuto::Calc(_) => true,
        ComputedLengthPercentageOrAuto::Auto => false,
    };
    [cv.margin.left, cv.margin.right]
        .into_iter()
        .any(nonzero_length_percentage_or_auto)
        || [cv.padding.left, cv.padding.right]
            .into_iter()
            .any(nonzero_length_percentage)
        || [cv.border.left.width().px(), cv.border.right.width().px()]
            .into_iter()
            .any(|width| width.abs() > f32::EPSILON)
}

/// Whether a joining-script shaping context may cross the selected inline
/// boundary: the first rendered character beyond the boundary must itself be
/// able to join (a joining-script character or an authored ZWJ), and no
/// box-model, atomic, line-break, or bidi-isolation boundary may intervene.
fn boundary_shaping_adjacent(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let mut node = idx;
    loop {
        let p = match parent_of[node] {
            Some(p) => p,
            None => return false,
        };
        let kids = &doc.nodes[p].children;
        let pos = match kids.iter().position(|&c| c == node) {
            Some(pos) => pos,
            None => return false,
        };
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(kids[..pos].iter().rev())
        } else {
            Box::new(kids[pos + 1..].iter())
        };
        for &sib in siblings {
            match boundary_inline_edge(doc, cascade, sib, dir, |_| false) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return false,
                InlineEdge::Char(ch) => {
                    return ch == '\u{200d}' || is_boundary_shaping_char(ch);
                }
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            if is_shaping_isolation_boundary(doc, p) || boundary_shaping_box_breaks(cascade, p) {
                return false;
            }
            node = p;
            continue;
        }
        return false;
    }
}

/// What a subtree contributes at the edge facing a shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineEdge {
    /// The subtree renders nothing inline; look further along the boundary.
    Empty,
    /// The subtree ends the joining context before any character.
    Break,
    /// The first rendered character met from the boundary side.
    Char(char),
}

/// Walk into `idx` from the side facing the boundary (`dir` +1 enters from
/// the start, -1 from the end) and report the first character it renders,
/// ignoring characters for which `skip` returns true. Out-of-flow and `display:none` boxes render nothing inline; atomic
/// inlines, `<br>`, bidi isolates, and inline boxes with nonzero inline-edge
/// margin/border/padding break the context.
fn boundary_inline_edge(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    dir: i8,
    skip: fn(char) -> bool,
) -> InlineEdge {
    let node = &doc.nodes[idx];
    if !node.is_in_document() {
        return InlineEdge::Empty;
    }
    if let Some(text) = text_of(doc, idx) {
        let edge = if dir < 0 {
            text.chars().rev().find(|&ch| !skip(ch))
        } else {
            text.chars().find(|&ch| !skip(ch))
        };
        return edge.map_or(InlineEdge::Empty, InlineEdge::Char);
    }
    // Comments and processing instructions never carry the in-document flag,
    // so any remaining node here is an element.
    let cv = &cascade.computed[idx];
    if cv.display == DisplayValue::None
        || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
    {
        return InlineEdge::Empty;
    }
    if !is_inline_for_trim(doc, cascade, idx)
        || is_shaping_isolation_boundary(doc, idx)
        || boundary_shaping_box_breaks(cascade, idx)
    {
        return InlineEdge::Break;
    }
    let children = &node.children;
    let ordered: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
        Box::new(children.iter().rev())
    } else {
        Box::new(children.iter())
    };
    for &child in ordered {
        match boundary_inline_edge(doc, cascade, child, dir, skip) {
            InlineEdge::Empty => continue,
            edge => return edge,
        }
    }
    InlineEdge::Empty
}

/// Raw text content of a text node.
fn text_of(doc: &Document, idx: usize) -> Option<&str> {
    match &doc.nodes[idx].data {
        crate::node::NodeData::Text(t) => Some(t.text_content.as_str()),
        _ => None,
    }
}

/// Collapse `text` per its computed `white-space` (CSS Text 3 §4.1) for
/// shaping: segment breaks/tabs become spaces (except `pre` family),
/// collapsible runs merge, and spaces at block boundaries are removed
/// (sibling context via `parent_of`, built once per preshape pass).
/// `pre`/`pre-wrap`/`break-spaces` pass through untouched; `pre-line`
/// keeps newlines but collapses spaces with boundary trimming.
/// Result of [`collapse_text_for_shaping`]: shaped text plus whether a
/// kept trailing space was stripped for forward migration.
pub(crate) struct CollapsedText {
    /// Text to shape (leading kept spaces already NBSP-ified in place;
    /// trailing kept spaces stripped — see `migrate_count`).
    pub(crate) text: String,
    /// A collapsible trailing space was removed while inline content
    /// follows: the caller prepends one NBSP to the next shaped text
    /// (parley keeps leading NBSP, trims trailing — so spaces only ever
    /// migrate forward, never stay trailing).
    pub(crate) migrate_count: u32,
}

fn has_authored_non_whitespace_text(doc: &Document, idx: usize) -> bool {
    text_of(doc, idx).is_some_and(|text| text.chars().any(|ch| !is_css_white_space(ch)))
        || doc.nodes[idx]
            .children
            .iter()
            .any(|&child| has_authored_non_whitespace_text(doc, child))
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn preserves_inline_whitespace_item(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    fallback_width: f32,
) -> bool {
    let Some(parent) = parent_of.get(idx).copied().flatten() else {
        return false;
    };
    if multicol_metrics_for_node(cascade, parent_of, parent, fallback_width).is_some()
        || !matches!(
            cascade.computed[parent].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        )
    {
        return false;
    }
    let Some(pos) = doc.nodes[parent]
        .children
        .iter()
        .position(|&child| child == idx)
    else {
        return false;
    };
    let has_inline_element = |children: &[usize]| {
        children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element
                && is_inline_element_box(cascade, child)
                && cascade.computed[child].display != DisplayValue::None
                && (has_authored_non_whitespace_text(doc, child)
                    // Replaced inline content has no text descendants.
                    || doc.nodes[child].tag_name() == Some("img"))
        })
    };
    has_inline_element(&doc.nodes[parent].children[..pos])
        && has_inline_element(&doc.nodes[parent].children[pos + 1..])
}

pub(crate) fn collapse_text_for_shaping(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    text: &str,
    ws: WhiteSpace,
) -> CollapsedText {
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
            return CollapsedText {
                text: text.to_string(),
                migrate_count: 0,
            };
        }
        _ => {}
    }
    // Flex/grid containers drop whitespace-only text children outright
    // (CSS Flexbox 1 §4 / CSS Grid 1 §5.2 anonymous-item rules: collapsed
    // runs vanish instead of becoming zero-size items). Without this the
    // migration below would inject NBSPs between flex items (e.g. WPT
    // flexbox_flex-0-0's newline-separated spans), shifting them apart.
    // Content-bearing text (anonymous flex/grid items) keeps flowing below.
    if text.chars().all(is_css_white_space)
        && let Some(p) = parent_of[idx]
        && doc.nodes[p].kind() == NodeKind::Element
        && matches!(
            cascade.computed[p].display,
            DisplayValue::Flex | DisplayValue::Grid
        )
    {
        return CollapsedText {
            text: String::new(),
            migrate_count: 0,
        };
    }
    // `matches!` carries its own wildcard arm, so this stays exhaustive
    // against the non-exhaustive `WhiteSpace` (cross-crate match without a
    // visible wildcard would not compile).
    // Phase I: CR/CRLF become LF; tabs/FF become spaces. Line feeds are
    // KEPT here (even for `normal`/`nowrap`): an interior single `\n`
    // carries East Asian Width / adjacency context that only the shaper
    // resolves correctly (CSS Text 3 §4.1.2 segment-break rules — parley
    // implements them; a blanket `\n`→space conversion breaks e.g. WPT
    // segment-break-transformation-rules-001 fullwidth/fullwidth). Runs of
    // 2+ line feeds collapse to one space below (multi-break runs never
    // reach the shaper as breaks; WPT removable-2 pins this), while a
    // single interior break passes through. Edge breaks are decided after
    // the boundary flags are known.
    let mut s = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            s.push('\n');
        } else if c == '\t' || c == '\x0C' {
            s.push(' ');
        } else {
            s.push(c);
        }
    }
    // Line-feed runs: `pre-line` keeps them verbatim for the shaper
    // (paragraph breaks survive); `normal`/`nowrap` collapse every maximal
    // run to one space here. The single-vs-wide decision for a lone break
    // between inlines happens in the all-whitespace arm below with East
    // Asian Width data; single-node content keeps the space approximation
    // it always had (multi-break collapse is pinned by WPT removable-2).
    let is_pre_line = matches!(ws, WhiteSpace::PreLine);
    // Lone-break optimized path (`normal`/`nowrap` only): an all-whitespace
    // node holding a segment-break run decides with direct-neighbor context
    // instead of the blanket conversion below. CSS Text 3 §4.1.2: a break
    // next to a zero-width space vanishes; next to a collapsible space the
    // space
    // survives (migrate); in a multi-break run one space survives; between
    // two wide (Fullwidth/Wide/Halfwidth, non-Hangul) chars it vanishes;
    // otherwise one space survives. `pre-line` keeps its feeds for the
    // shaper via the normal pipeline (dedupe carve-out above). Mixed text
    // nodes are handled by the removable-run pre-pass below.
    if !is_pre_line
        && s.chars().all(is_css_white_space)
        && whitespace_run_has_break(doc, cascade, parent_of, idx, text)
    {
        // A segment-break run may be split into several whitespace text
        // nodes by the HTML tokenizer. Let only its first node decide the
        // result; otherwise each LF would independently migrate a space.
        if edge_char(doc, cascade, parent_of, idx, -1).is_some_and(is_css_white_space) {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        let before = has_inline_adjacent(doc, cascade, parent_of, idx, -1);
        let after = has_inline_adjacent(doc, cascade, parent_of, idx, 1);
        if !before || !after {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        // Skip all CSS whitespace nodes surrounding this run before applying
        // the segment-break transformation rules. In particular, removable-3
        // and removable-4 put ordinary spaces around every LF; those spaces
        // are part of the same sequence and must not migrate before the wide
        // character test runs.
        let p = edge_non_whitespace_char(doc, cascade, parent_of, idx, -1);
        let n = edge_non_whitespace_char(doc, cascade, parent_of, idx, 1);
        if p == Some(ZERO_WIDTH_SPACE) || n == Some(ZERO_WIDTH_SPACE) {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        if let (Some(a), Some(b)) = (p, n)
            && is_wide_for_break(a)
            && is_wide_for_break(b)
        {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        return CollapsedText {
            text: String::new(),
            migrate_count: 1,
        };
    }

    let s = if is_pre_line {
        s
    } else {
        remove_removable_segment_break_runs(doc, cascade, parent_of, idx, &s)
    };
    let chars_vec: Vec<char> = s.chars().collect();
    let mut merged = String::with_capacity(s.len());
    let mut in_spaces = false;
    let mut i = 0usize;
    while i < chars_vec.len() {
        let c = chars_vec[i];
        if c == ' ' {
            if !in_spaces {
                merged.push(' ');
            }
            in_spaces = true;
            i += 1;
        } else if c == '\n' {
            let mut j = i;
            while j < chars_vec.len() && chars_vec[j] == '\n' {
                j += 1;
            }
            if is_pre_line {
                for &k in &chars_vec[i..j] {
                    merged.push(k);
                }
                in_spaces = false;
            } else {
                merged.push(' ');
                in_spaces = true;
            }
            i = j;
        } else {
            in_spaces = false;
            merged.push(c);
            i += 1;
        }
    }
    // Extended dedupe: an all-whitespace node whose previous relevant
    // sibling's raw text ENDS with whitespace collapses away — runs
    // spanning nodes (spaces, breaks, or mixed) keep the single space the
    // earlier node migrates. Skipped for `pre-line` around line feeds (a
    // following break must survive to shape the paragraph); pure-space
    // runs still dedupe.
    // NBSP-bearing nodes never dedupe: NBSPs don't collapse, so each
    // migrating NBSP node adds its own space to the pending count.
    if merged.chars().all(is_css_white_space)
        && !merged.contains('\u{00A0}')
        && let Some(p) = parent_of[idx]
        && let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == idx)
    {
        let raw_has_break = text_of(doc, idx).is_some_and(|t| t.contains('\n'));
        for &prev in doc.nodes[p].children[..pos].iter().rev() {
            if !doc.nodes[prev].is_in_document() {
                continue;
            }
            if doc.nodes[prev].kind() == NodeKind::Element
                && cascade.computed[prev].display == DisplayValue::None
            {
                continue;
            }
            if doc.nodes[prev].kind() == NodeKind::Text
                && let Some(t) = text_of(doc, prev)
            {
                let ends_ws = t.chars().next_back().is_some_and(is_css_white_space);
                if !ends_ws {
                    break;
                }
                if is_pre_line && (raw_has_break || t.contains('\n')) {
                    break;
                }
                // Run continues here: the earlier node migrates the
                // single surviving space.
                return CollapsedText {
                    text: String::new(),
                    migrate_count: 0,
                };
            }
            break;
        }
    }
    let before = has_inline_adjacent(doc, cascade, parent_of, idx, -1);
    let after = has_inline_adjacent(doc, cascade, parent_of, idx, 1);
    // Lone-space arm covers plain spaces AND lone NBSPs (a `&nbsp;` entity
    // parses to its own text node, which parley would otherwise trim to
    // zero when shaped alone — the same fate as a lone collapsible space,
    // so it migrates identically; WPT unremovable-* pins the outcome).
    if merged.chars().all(|c| c == ' ' || c == '\u{00A0}') {
        // Fully collapsed: a lone boundary space vanishes; between inlines
        // the survivors migrate forward (shaped alone even NBSP trims to
        // zero — parley keeps only leading non-collapsible runs followed
        // by more content, measured directly). Count = NBSPs plus one for
        // a collapsible run (runs already merged to one above; NBSPs never
        // collapse, so each is preserved).
        if before && after {
            let count = merged.chars().filter(|&c| c == '\u{00A0}').count() as u32
                + merged.contains(' ') as u32;
            return CollapsedText {
                text: String::new(),
                migrate_count: count,
            };
        }
        return CollapsedText {
            text: String::new(),
            migrate_count: 0,
        };
    }
    // Edge line feeds (single — runs already collapsed above): dropped
    // at block edges, converted to a space toward inline content (a lone
    // survivor is indistinguishable from a collapsed space downstream;
    // wide-char pairs across nodes stay space-approximated — symmetric
    // pairs are unaffected either way).
    let mut out = merged;
    if !before {
        out = out.trim_start_matches([' ', '\n']).to_string();
    } else if out.starts_with('\n') {
        out.replace_range(..1, " ");
    }
    if !after {
        out = out.trim_end_matches([' ', '\n']).to_string();
    } else if out.ends_with('\n') {
        out.pop();
        out.push(' ');
    }
    let trailing_kept = after && out.ends_with(' ');
    if after {
        // Trailing survivor migrates forward (see `migrate_count`); strip
        // it here so shaping never sees a trimmable edge space.
        out = out.trim_end_matches(' ').to_string();
    }
    // Kept leading spaces become NBSP in place: parley keeps a leading
    // NBSP followed by content (measured), while a plain leading space
    // would trim. NBSP forfeits a soft-wrap opportunity at that spot;
    // acceptable since the alternative (today) is losing the space.
    if out.starts_with(' ') {
        out.replace_range(..1, "\u{00A0}");
    }
    CollapsedText {
        text: out,
        migrate_count: trailing_kept as u32,
    }
}

/// A coarse CSS Text 4 autospace class.  The style layer preserves the
/// complete `text-autospace` value; this layout slice only needs the boundary
/// classes to create non-painting in-flow advances.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextAutospaceClass {
    Ideograph,
    Letter,
    Numeric,
    Other,
}

fn text_autospace_class(c: char) -> TextAutospaceClass {
    use icu_properties::props::{GeneralCategoryGroup, Script};

    let gc =
        icu_properties::CodePointMapDataBorrowed::<icu_properties::props::GeneralCategory>::new()
            .get(c);
    let eaw = east_asian_width_map().get(c);
    let wide = matches!(
        eaw,
        icu_properties::props::EastAsianWidth::Fullwidth
            | icu_properties::props::EastAsianWidth::Wide
    );
    // CSS Text's ideographic set includes Han and the Japanese kana/stroke
    // ranges. Punctuation in the shared ranges is not an ideograph.
    let ideograph =
        (icu_properties::CodePointMapDataBorrowed::<icu_properties::props::Script>::new().get(c)
            == Script::Han
            || matches!(c, '\u{3041}'..='\u{30ff}' | '\u{31c0}'..='\u{31ff}'))
            && !GeneralCategoryGroup::Punctuation.contains(gc);
    if ideograph {
        return TextAutospaceClass::Ideograph;
    }
    if !wide
        && (GeneralCategoryGroup::Letter.contains(gc) || GeneralCategoryGroup::Mark.contains(gc))
    {
        return TextAutospaceClass::Letter;
    }
    if !wide && GeneralCategoryGroup::Number.contains(gc) {
        return TextAutospaceClass::Numeric;
    }
    TextAutospaceClass::Other
}

fn text_autospace_ascii_punctuation(c: char) -> bool {
    // CSS Text 4's `punctuation` class is language-sensitive. Chromium's
    // current behavior (and the zh WPT slice) inserts around these ASCII
    // punctuation marks, but not around fullwidth/CJK punctuation.
    matches!(c, '!' | '#' | ':' | ';' | '?')
}

#[cfg(test)]
pub(crate) fn text_autospace_boxes(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_edges(text, value, language, font_size, None, None)
}

fn text_autospace_boxes_with_edges(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_width(
        text,
        value,
        language,
        (font_size * 0.125).max(0.0),
        before,
        after,
    )
}

fn text_autospace_boxes_with_width(
    text: &str,
    value: TextAutospace,
    language: &str,
    width: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    let (ideograph_alpha, ideograph_numeric, punctuation) = match value {
        TextAutospace::Normal | TextAutospace::Auto => {
            (true, true, language_matches(language, "zh"))
        }
        TextAutospace::NoAutospace => (false, false, false),
        TextAutospace::Custom {
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
            ..
        } => (
            ideograph_alpha,
            ideograph_numeric,
            punctuation && language_matches(language, "zh"),
        ),
        // `TextAutospace` is non-exhaustive so downstream crates remain
        // source-compatible when the style layer gains another keyword.
        _ => (true, true, language_matches(language, "zh")), // cov:ignore: no future non-exhaustive variant exists in the pinned style crate.
    };
    if !ideograph_alpha && !ideograph_numeric && !punctuation {
        return Vec::new();
    }
    if !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }

    let mut previous =
        before.and_then(|c| (!is_segment_break_ignorable(c)).then(|| (c, text_autospace_class(c))));
    let mut last = None;
    let mut boxes = Vec::new();
    for (index, c) in text.char_indices() {
        // Default-ignorable characters, including variation selectors, do not
        // break the neighboring-character test. The VS WPT expects `国` + VS
        // + `A` to use the same autospace boundary as `国A`.
        if is_segment_break_ignorable(c) {
            continue;
        }
        let class = text_autospace_class(c);
        if let Some((previous_char, previous_class)) = previous
            && text_autospace_pair_needs_box(
                previous_char,
                previous_class,
                c,
                class,
                ideograph_alpha,
                ideograph_numeric,
                punctuation,
            )
        {
            boxes.push(InlineBox {
                // Reserve the high ID range for autospace boxes so paint can
                // distinguish their inline baseline semantics from tabs.
                id: u64::MAX - boxes.len() as u64,
                kind: InlineBoxKind::InFlow,
                index,
                width,
                height: 0.0,
            });
        }
        previous = Some((c, class));
        last = Some((c, class));
    }
    if let (Some((previous_char, previous_class)), Some(next)) = (last, after)
        && !is_segment_break_ignorable(next)
        && text_autospace_pair_needs_box(
            previous_char,
            previous_class,
            next,
            text_autospace_class(next),
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
        )
    {
        boxes.push(InlineBox {
            // Reserve the high ID range for autospace boxes so paint can
            // distinguish their inline baseline semantics from tabs.
            id: u64::MAX - boxes.len() as u64,
            kind: InlineBoxKind::InFlow,
            index: text.len(),
            width,
            height: 0.0,
        });
    }
    boxes
}

fn text_autospace_pair_needs_box(
    previous_char: char,
    previous_class: TextAutospaceClass,
    current_char: char,
    current_class: TextAutospaceClass,
    ideograph_alpha: bool,
    ideograph_numeric: bool,
    punctuation: bool,
) -> bool {
    let class_boundary = match (previous_class, current_class) {
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Letter)
        | (TextAutospaceClass::Letter, TextAutospaceClass::Ideograph) => ideograph_alpha,
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Numeric)
        | (TextAutospaceClass::Numeric, TextAutospaceClass::Ideograph) => ideograph_numeric,
        _ => false,
    };
    let punctuation_boundary = punctuation
        && ((text_autospace_ascii_punctuation(previous_char)
            && current_class == TextAutospaceClass::Ideograph)
            || (previous_class == TextAutospaceClass::Ideograph
                && text_autospace_ascii_punctuation(current_char)));
    class_boundary || punctuation_boundary
}

fn effective_language_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_idx: usize,
) -> String {
    let mut current = parent_of[text_idx];
    while let Some(idx) = current {
        if let crate::node::NodeData::Element(element) = &doc.nodes[idx].data
            && let Some(attr) = element
                .attributes
                .iter()
                .find(|attr| attr.local.eq_ignore_ascii_case("lang") || attr.local == "xml:lang")
        {
            return attr.value.trim().to_ascii_lowercase();
        }
        current = parent_of[idx];
    }
    String::new()
}

fn language_matches(language: &str, primary: &str) -> bool {
    language == primary
        || language
            .strip_prefix(primary)
            .is_some_and(|rest| rest.starts_with('-'))
}

fn text_transform_has_full_width(transform: TextTransform) -> bool {
    matches!(
        transform,
        TextTransform::FullWidth
            | TextTransform::CapitalizeFullWidth
            | TextTransform::UppercaseFullWidth
            | TextTransform::LowercaseFullWidth
            | TextTransform::FullWidthFullSizeKana
            | TextTransform::CapitalizeFullWidthFullSizeKana
            | TextTransform::UppercaseFullWidthFullSizeKana
            | TextTransform::LowercaseFullWidthFullSizeKana
    )
}

/// Apply the supported CSS `text-transform` values after whitespace collapsing
/// and before shaping. The operation is performed on each text node, preserving
/// the existing layout and paint ownership model.
fn apply_text_transform(text: &str, transform: TextTransform, language: &str) -> String {
    let (case, full_width, full_size_kana) = match transform {
        TextTransform::None => (None, false, false),
        TextTransform::Capitalize => (Some(TextTransform::Capitalize), false, false),
        TextTransform::Uppercase => (Some(TextTransform::Uppercase), false, false),
        TextTransform::Lowercase => (Some(TextTransform::Lowercase), false, false),
        TextTransform::FullWidth => (None, true, false),
        TextTransform::FullSizeKana => (None, false, true),
        TextTransform::CapitalizeFullWidth => (Some(TextTransform::Capitalize), true, false),
        TextTransform::UppercaseFullWidth => (Some(TextTransform::Uppercase), true, false),
        TextTransform::LowercaseFullWidth => (Some(TextTransform::Lowercase), true, false),
        TextTransform::CapitalizeFullSizeKana => (Some(TextTransform::Capitalize), false, true),
        TextTransform::UppercaseFullSizeKana => (Some(TextTransform::Uppercase), false, true),
        TextTransform::LowercaseFullSizeKana => (Some(TextTransform::Lowercase), false, true),
        TextTransform::FullWidthFullSizeKana => (None, true, true),
        TextTransform::CapitalizeFullWidthFullSizeKana => {
            (Some(TextTransform::Capitalize), true, true)
        }
        TextTransform::UppercaseFullWidthFullSizeKana => {
            (Some(TextTransform::Uppercase), true, true)
        }
        TextTransform::LowercaseFullWidthFullSizeKana => {
            (Some(TextTransform::Lowercase), true, true)
        }
        _ => (None, false, false),
    };
    let mut cased = String::with_capacity(text.len());
    let mut in_word = false;
    for c in text.chars() {
        match case {
            Some(TextTransform::Uppercase) => {
                if language_matches(language, "tr") || language_matches(language, "az") {
                    match c {
                        'i' => cased.push('İ'),
                        'ı' => cased.push('I'),
                        _ => cased.extend(c.to_uppercase()),
                    }
                } else {
                    cased.extend(c.to_uppercase());
                }
            }
            Some(TextTransform::Lowercase) => {
                if language_matches(language, "tr") || language_matches(language, "az") {
                    match c {
                        'I' => cased.push('ı'),
                        'İ' => cased.push('i'),
                        _ => cased.extend(c.to_lowercase()),
                    }
                } else {
                    cased.extend(c.to_lowercase());
                }
            }
            Some(TextTransform::Capitalize) => {
                if c.is_alphanumeric() {
                    let can_titlecase = !(0x24D0..=0x24E9).contains(&(c as u32));
                    if !in_word && can_titlecase {
                        cased.extend(c.to_uppercase());
                    } else {
                        cased.push(c);
                    }
                    in_word = true;
                } else {
                    cased.push(c);
                    in_word = false;
                }
            }
            _ => cased.push(c),
        }
    }
    if matches!(case, Some(TextTransform::Lowercase))
        && (language_matches(language, "tr") || language_matches(language, "az"))
    {
        tailor_turkic_combining_dot(&mut cased);
    }
    if matches!(case, Some(TextTransform::Capitalize)) && language_matches(language, "nl") {
        tailor_dutch_ij(&mut cased);
    }
    if matches!(case, Some(TextTransform::Uppercase)) && language_matches(language, "el") {
        strip_greek_tonos(&mut cased);
    }
    cased
        .chars()
        .map(|c| {
            let c = if full_size_kana {
                full_size_kana_char(c)
            } else {
                c
            };
            if full_width { full_width_char(c) } else { c }
        })
        .collect()
}

fn tailor_turkic_combining_dot(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let combining_dot = char::from_u32(0x0307).unwrap();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == 'ı' && chars.get(index + 1) == Some(&combining_dot) {
            out.push('i');
            index += 2;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    *text = out;
}

fn tailor_dutch_ij(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if at_word_start && c == 'I' && chars.get(index + 1) == Some(&'j') {
            out.push('I');
            out.push('J');
            at_word_start = false;
            index += 2;
            continue;
        }
        out.push(c);
        at_word_start = !c.is_alphanumeric();
        index += 1;
    }
    *text = out;
}

fn strip_greek_tonos(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_alphabetic() {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let mut end = index;
        while end < chars.len()
            && (chars[end].is_alphabetic() || ('\u{0300}'..='\u{036f}').contains(&chars[end]))
        {
            end += 1;
        }
        let letter_count = chars[index..end]
            .iter()
            .filter(|c| c.is_alphabetic())
            .count();
        let keep_tonos = letter_count == 1;
        let mut word_index = index;
        while word_index < end {
            let c = chars[word_index];
            if !keep_tonos
                && let Some(&next) = chars.get(word_index + 1)
                && matches!(c, 'Ά' | 'Έ' | 'Ή' | 'Ί' | 'Ό' | 'Ύ' | 'Ώ')
                && matches!(next, 'Ι' | 'Υ')
            {
                out.push(greek_tonos_removed(c));
                out.push(greek_diaeresis(next));
                word_index += 2;
            } else if !keep_tonos
                && let (Some(&diaeresis), Some(&acute)) =
                    (chars.get(word_index + 1), chars.get(word_index + 2))
                && matches!(c, 'Ι' | 'Υ')
                && diaeresis == '\u{0308}'
                && acute == '\u{0301}'
            {
                out.push(greek_diaeresis(c));
                word_index += 3;
            } else if !keep_tonos && c == '\u{0301}' {
                word_index += 1;
            } else {
                out.push(if keep_tonos {
                    c
                } else {
                    greek_tonos_removed(c)
                });
                word_index += 1;
            }
        }
        index = end;
    }
    *text = out;
}

fn greek_tonos_removed(c: char) -> char {
    match c {
        'Ά' => 'Α',
        'Έ' => 'Ε',
        'Ή' => 'Η',
        'Ί' => 'Ι',
        'Ό' => 'Ο',
        'Ύ' => 'Υ',
        'Ώ' => 'Ω',
        _ => c,
    }
}

fn greek_diaeresis(c: char) -> char {
    match c {
        'Ι' => 'Ϊ',
        'Υ' => 'Ϋ',
        _ => c,
    }
}

fn full_size_kana_char(c: char) -> char {
    match c {
        'ぁ' => 'あ',
        'ぃ' => 'い',
        'ぅ' => 'う',
        'ぇ' => 'え',
        'ぉ' => 'お',
        'ゕ' => 'か',
        'ゖ' => 'け',
        'っ' => 'つ',
        'ゃ' => 'や',
        'ゅ' => 'ゆ',
        'ょ' => 'よ',
        'ゎ' => 'わ',
        'ァ' => 'ア',
        'ィ' => 'イ',
        'ゥ' => 'ウ',
        'ェ' => 'エ',
        'ォ' => 'オ',
        'ヵ' => 'カ',
        'ㇰ' => 'ク',
        'ヶ' => 'ケ',
        'ㇱ' => 'シ',
        'ㇲ' => 'ス',
        'ッ' => 'ツ',
        'ㇳ' => 'ト',
        'ㇴ' => 'ヌ',
        'ㇵ' => 'ハ',
        'ㇶ' => 'ヒ',
        'ㇷ' => 'フ',
        'ㇸ' => 'ヘ',
        'ㇹ' => 'ホ',
        'ㇺ' => 'ム',
        'ャ' => 'ヤ',
        'ュ' => 'ユ',
        'ョ' => 'ヨ',
        'ㇻ' => 'ラ',
        'ㇼ' => 'リ',
        'ㇽ' => 'ル',
        'ㇾ' => 'レ',
        'ㇿ' => 'ロ',
        'ヮ' => 'ワ',
        'ｧ' => 'ｱ',
        'ｨ' => 'ｲ',
        'ｩ' => 'ｳ',
        'ｪ' => 'ｴ',
        'ｫ' => 'ｵ',
        'ｯ' => 'ﾂ',
        'ｬ' => 'ﾔ',
        'ｭ' => 'ﾕ',
        'ｮ' => 'ﾖ',
        '\u{1B132}' => '\u{3053}',
        '\u{1B150}' => '\u{3090}',
        '\u{1B151}' => '\u{3091}',
        '\u{1B152}' => '\u{3092}',
        '\u{1B155}' => '\u{30B3}',
        '\u{1B164}' => '\u{30F0}',
        '\u{1B165}' => '\u{30F1}',
        '\u{1B166}' => '\u{30F2}',
        '\u{1B167}' => '\u{30F3}',
        _ => c,
    }
}

fn full_width_char(c: char) -> char {
    match c {
        ' ' => char::from_u32(0x3000).unwrap(),
        '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
        _ => c,
    }
}

fn is_leading_body_text(document: &Document, body_id: Option<usize>, node_id: usize) -> bool {
    let Some(body_id) = body_id else {
        return false;
    };
    for &child_id in &document.nodes[body_id].children {
        if child_id == node_id {
            return true;
        }
        let Some(child) = document.get_node(child_id) else {
            continue;
        };
        match child.kind() {
            NodeKind::Text => {
                if let crate::node::NodeData::Text(text) = &child.data
                    && !text.text_content.trim().is_empty()
                {
                    return false;
                }
            }
            NodeKind::Element
                if child.is_display_none() || child.is_non_rendered_html_element() => {}
            _ => return false,
        }
    }
    false
}

fn parley_word_break(value: WordBreak, _line_break: LineBreak) -> ParleyWordBreak {
    match value {
        WordBreak::BreakAll => ParleyWordBreak::BreakAll,
        WordBreak::KeepAll => ParleyWordBreak::KeepAll,
        // `manual`, `auto-phrase`, and the deprecated `break-word` do not
        // have a direct Parley word-break mode. `break-word` gets its
        // emergency wrapping behavior from `parley_overflow_wrap` below.
        WordBreak::Normal | WordBreak::Manual | WordBreak::AutoPhrase | WordBreak::BreakWord => {
            ParleyWordBreak::Normal
        }
        _ => ParleyWordBreak::Normal,
    }
}

fn has_visible_non_nbsp_character(text: &str) -> bool {
    text.chars()
        .any(|character| character != '\u{00A0}' && !is_css_white_space(character))
}

// The callback overrides every analyzed UAX line boundary. Keep control
// classes without exact WPT coverage on the per-job fallback path.
fn line_break_class_map()
-> icu_properties::CodePointMapDataBorrowed<'static, icu_properties::props::LineBreak> {
    icu_properties::CodePointMapDataBorrowed::<icu_properties::props::LineBreak>::new()
}

fn has_unverified_anywhere_control(text: &str) -> bool {
    use icu_properties::props::LineBreak as UnicodeLineBreak;

    let classes = line_break_class_map();
    text.chars().any(|character| {
        matches!(
            classes.get(character),
            UnicodeLineBreak::Glue
                | UnicodeLineBreak::WordJoiner
                | UnicodeLineBreak::ZWSpace
                | UnicodeLineBreak::ZWJ
                | UnicodeLineBreak::CombiningMark
        ) && !matches!(
            character,
            '\u{00A0}' // verified WPT -006/-010; replaced by the Raikiri adapter
                | '\u{2060}' // verified WPT overrides-uax-behavior-001
                | '\u{FEFF}' // verified WPT overrides-uax-behavior-002
                | '\u{200B}' // verified WPT overrides-uax-behavior-003
                | '\u{180E}' // verified WPT overrides-uax-behavior-010
                | '\u{034F}' // verified WPT overrides-uax-behavior-012
                | '\u{200D}' // verified WPT overrides-uax-behavior-015
        )
    })
}

fn pre_wrap_anywhere_subset(text: &str) -> bool {
    text.matches('\u{00A0}').count() == 1
        && text
            .chars()
            .all(|character| !is_css_white_space(character) || character == ' ')
        && !text.starts_with(' ')
        && !text.ends_with(' ')
        && !text.contains("  ")
        && has_visible_non_nbsp_character(text)
}

fn should_enable_anywhere_override(
    line_break: LineBreak,
    white_space: WhiteSpace,
    nowrap: bool,
    text: &str,
) -> bool {
    if line_break != LineBreak::Anywhere
        || nowrap
        || !has_visible_non_nbsp_character(text)
        || has_unverified_anywhere_control(text)
    {
        return false;
    }

    match white_space {
        WhiteSpace::Normal | WhiteSpace::PreLine => true,
        WhiteSpace::PreWrap => pre_wrap_anywhere_subset(text),
        _ => false,
    }
}

fn should_enable_anywhere_callback(
    line_break: LineBreak,
    white_space: WhiteSpace,
    nowrap: bool,
    text: &str,
    nbsp_adapter_enabled: bool,
) -> bool {
    should_enable_anywhere_override(line_break, white_space, nowrap, text)
        && (!text.contains('\u{00A0}') || nbsp_adapter_enabled)
}

fn should_enable_nbsp_adapter(
    line_break: LineBreak,
    white_space: WhiteSpace,
    nowrap: bool,
    text: &str,
    has_authored_tab: bool,
    has_inline_root_ancestor: bool,
) -> bool {
    !(has_inline_root_ancestor || (white_space == WhiteSpace::PreWrap && has_authored_tab))
        && text.contains('\u{00A0}')
        && should_enable_anywhere_override(line_break, white_space, nowrap, text)
}

fn has_inline_root_ancestor(doc: &Document, parent_of: &[Option<usize>], node: usize) -> bool {
    let mut ancestor = parent_of[node];
    while let Some(index) = ancestor {
        if doc.nodes[index].flags.contains(NodeFlags::IS_INLINE_ROOT) {
            return true;
        }
        ancestor = parent_of[index];
    }
    false
}

fn line_break_anywhere_override(context: parley::LineBreakContext) -> Option<bool> {
    // A mandatory newline already ends the current line. Do not create a
    // second soft opportunity immediately before it.
    Some(context.after != '\n')
}

static LINE_BREAK_ANYWHERE_OVERRIDE: &parley::LineBreakOverrideFn =
    &(line_break_anywhere_override as fn(parley::LineBreakContext) -> Option<bool>);

fn parley_line_break_override(enabled: bool) -> Option<&'static parley::LineBreakOverrideFn> {
    enabled.then_some(LINE_BREAK_ANYWHERE_OVERRIDE)
}

fn parley_overflow_wrap(
    word_break: WordBreak,
    _line_break: LineBreak,
    value: OverflowWrap,
) -> ParleyOverflowWrap {
    if matches!(word_break, WordBreak::BreakWord) {
        return ParleyOverflowWrap::BreakWord;
    }
    match value {
        OverflowWrap::Normal => ParleyOverflowWrap::Normal,
        OverflowWrap::Anywhere => ParleyOverflowWrap::Anywhere,
        OverflowWrap::BreakWord => ParleyOverflowWrap::BreakWord,
        _ => ParleyOverflowWrap::Normal,
    }
}

fn parley_text_wrap_mode(value: TextWrapMode, nowrap: bool) -> ParleyTextWrapMode {
    if nowrap || matches!(value, TextWrapMode::Nowrap) {
        ParleyTextWrapMode::NoWrap
    } else {
        ParleyTextWrapMode::Wrap
    }
}

pub(crate) fn preshape_text(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
    page_width: f32,
) {
    for node in &mut doc.nodes {
        if let Some(text) = node.data.as_text_mut() {
            text.snap_glyph_x_to_1_64 = false;
        }
    }
    // Cheap threshold: for tiny DOMs sequential is faster than rayon overhead.
    // Collect eligible Text nodes first to avoid borrowing `doc.nodes` mutably
    // across parallel tasks (which would require Sync on Document). Owned jobs
    // are Send and avoid sharing &Document across threads.
    struct Job {
        idx: usize,
        had_tab: bool,
        text: String,
        family_str: String,
        font_size_raw: f32,
        font_weight_raw: f32,
        font_style: StyleFontStyle,
        line_height_raw: ComputedLineHeight,
        letter_spacing_raw: f32,
        // Preserve authored `ch` so the shaping font's `0` advance can replace
        // the style-layer fallback before Parley lays out the text.
        letter_spacing_ch_factor: Option<f32>,
        // Style-layer fallback in CSS px; replaced with a measured `ch`
        // advance when the authored-unit marker below is present.
        word_spacing_raw: f32,
        // Preserve the authored unit because `ComputedLength` alone loses it.
        word_spacing_ch_factor: Option<f32>,
        tab_size: ComputedTabSize,
        white_space: WhiteSpace,
        word_break: WordBreak,
        line_break: LineBreak,
        line_break_override_enabled: bool,
        has_inline_root_ancestor: bool,
        overflow_wrap: OverflowWrap,
        text_wrap_mode: TextWrapMode,
        autospace_boxes: Vec<InlineBox>,
        // Soft wrapping suppressed (`white-space: nowrap` or
        // `text-wrap: nowrap`).
        nowrap: bool,
        max_advance: f32,
        // tab-stop metrics use the block-container ancestor's font and spacing.
        metrics_family: String,
        metrics_size: f32,
        metrics_weight: f32,
        metrics_style: StyleFontStyle,
        metrics_letter_spacing_raw: f32,
        metrics_letter_spacing_ch_factor: Option<f32>,
        metrics_word_spacing_raw: f32,
        metrics_word_spacing_ch_factor: Option<f32>,
        simple_pre_block: bool,
        // Preserve the metric quantization used by a simple pre block when
        // one inline wrapper contains the only text run and a terminal
        // preserved newline is the sole sibling.
        simple_preserved_run: bool,
        tab_spacing_ranges: Vec<(std::ops::Range<usize>, f32)>,
    }

    /// probe cache key: (family, size bits, weight bits, style discriminant)。
    /// tab-stop の metrics は block-container 祖先の font で取る
    /// (CSS Text 3 §4.2 — integer-004 / block-ancestor が pin)。
    /// shape 自体の font (own) とは別物であることに注意。
    fn font_key(job: &Job) -> (String, u32, u32, u8) {
        let style = match job.metrics_style {
            StyleFontStyle::Normal => 0,
            StyleFontStyle::Italic => 1,
            StyleFontStyle::Oblique => 2,
            _ => 0,
        };
        (
            job.metrics_family.clone(),
            job.metrics_size.to_bits(),
            job.metrics_weight.to_bits(),
            style,
        )
    }

    fn shape_font_key(job: &Job) -> (String, u32, u32, u8) {
        let style = match job.font_style {
            StyleFontStyle::Normal => 0,
            StyleFontStyle::Italic => 1,
            StyleFontStyle::Oblique => 2,
            _ => 0,
        };
        (
            job.family_str.clone(),
            job.font_size_raw.to_bits(),
            job.font_weight_raw.to_bits(),
            style,
        )
    }

    // Shared parent map for simple pre-block eligibility and whitespace
    // boundary trimming (the arena has no stored parent pointers).
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in doc.nodes[idx].children.clone().iter() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    fn family_str_of(cv: &ComputedValues) -> String {
        cv.font_family
            .iter()
            .map(|a| a.0.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
    let mut jobs: Vec<Job> = Vec::with_capacity(doc.nodes.len() / 2);
    // CSS Text 4 `ic` probes are shared by text nodes with the same font
    // selection, just like the existing `ch` probe cache below.
    let mut ic_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    // Outstanding forward-migrated spaces (counted: collapsible space
    // runs collapse to one via dedupe, but NBSPs never collapse so each
    // migrating NBSP node adds one).
    let mut migrate_pending: u32 = 0;
    let mut migrate_pending_full_width = false;
    let body_id = (0..doc.nodes.len()).find(|&idx| doc.nodes[idx].tag_name() == Some("body"));
    let has_out_of_flow_ancestor = |mut parent: Option<usize>| {
        while let Some(id) = parent {
            if matches!(
                cascade.computed[id].position,
                PositionValue::Absolute | PositionValue::Fixed
            ) {
                return true;
            }
            parent = parent_of[id];
        }
        false
    };
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        let raw: String = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(t) if !t.text_content.is_empty() => {
                t.text_content.as_str().to_string()
            }
            _ => continue,
        };

        // CSS Text 3 §4.1: collapse segment breaks/runs and trim boundary
        // spaces before shaping. Fully-collapsed text keeps `text_layout`
        // empty (`None`, same as empty source text above): shaping `""`
        // would still produce a strut line, but collapsed-away text must
        // contribute zero size. Paint skips `None` layouts.
        // `display:none` text is skipped entirely (neither shaped nor
        // migrating): it generates no boxes, so it must not consume a
        // pending migrated space.
        if cascade.computed[idx].display == DisplayValue::None {
            continue;
        }
        let cv = &cascade.computed[idx];
        let language = effective_language_for_text(doc, &parent_of, idx);
        let ws = cv.white_space;
        let collapsed = collapse_text_for_shaping(doc, cascade, &parent_of, idx, &raw, ws);
        if std::env::var("COLLAPSE_DBG2").is_ok() {
            eprintln!(
                "C2 raw={:?} out={:?} mig={}",
                raw, collapsed.text, collapsed.migrate_count
            );
        }
        // A space migrated by an EARLIER node lands here (never this
        // node's own trailing space, which belongs after it).
        let pending_in = migrate_pending;
        let mut text = collapsed.text;
        if text.is_empty() {
            // Whitespace separators between inline children become flex-item
            // boundaries in the multicol projection, so they must not migrate
            // into the next child run (table-cell references have the same
            // boundary behavior).
            // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
            let whitespace_boundary = parent_of[idx].is_some_and(|parent| {
                multicol_metrics_for_node(cascade, &parent_of, parent, max_advance).is_some()
            });
            // cov:ignore: whitespace boundary migration is exercised by the ignored foundation WPT run.
            if whitespace_boundary {
                migrate_pending = 0;
                migrate_pending_full_width = false;
                continue;
            }
            // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
            let inline_space_count =
                // cov:ignore: preserved inline whitespace sizing is exercised by the ignored foundation WPT run.
                if preserves_inline_whitespace_item(doc, cascade, &parent_of, idx, max_advance) {
                    let nbsp_count = raw.chars().filter(|&ch| ch == '\u{00a0}').count() as u32;
                    if nbsp_count > 0 {
                        nbsp_count
                    } else if raw.chars().any(is_css_white_space) {
                        1
                    } else {
                        0
                    }
                } else {
                    0
                };
            // cov:ignore: preserved inline whitespace sizing is exercised by the ignored foundation WPT run.
            if inline_space_count > 0 {
                let family = family_str_of(cv);
                let space_advance = probe_text_full_width(
                    fonts,
                    layout_cx,
                    " ",
                    TextProbeStyle {
                        family_str: &family,
                        font_size_px: cv.font_size.px(),
                        font_weight: cv.font_weight,
                        font_style: cv.font_style,
                        letter_spacing: cv.letter_spacing.px(),
                        word_spacing: cv.word_spacing.px(),
                    },
                );
                doc.nodes[idx].style.size.width =
                    Dimension::length(space_advance * inline_space_count as f32);
                migrate_pending = 0;
                migrate_pending_full_width = false;
                continue;
            }
            // A collapsed space styled with `full-width` must remain owned by
            // that inline run: attaching its transformed U+3000 to the
            // preceding job avoids changing the line metrics of the following
            // text node while retaining the source style's transform.
            if collapsed.migrate_count > 0
                && pending_in == 0
                && text_transform_has_full_width(cv.text_transform)
                && has_inline_adjacent(doc, cascade, &parent_of, idx, -1)
                && has_inline_adjacent(doc, cascade, &parent_of, idx, 1)
                && let Some(previous) = jobs.last_mut()
            {
                previous.text.push('\u{3000}');
                continue;
            }
            // Empty nodes neither consume nor (unless migrating themselves)
            // clear outstanding spaces: a boundary-dropped node must not
            // cancel earlier migrations.
            migrate_pending = pending_in.saturating_add(collapsed.migrate_count);
            if collapsed.migrate_count > 0 {
                migrate_pending_full_width = text_transform_has_full_width(cv.text_transform);
            }
            continue;
        }
        let pending_full_width = migrate_pending_full_width;
        migrate_pending = collapsed.migrate_count;
        migrate_pending_full_width =
            collapsed.migrate_count > 0 && text_transform_has_full_width(cv.text_transform);
        // Forward-migrated spaces (see `migrate_space`): prepend NBSPs iff
        // they still precede inline content from THIS node's edge — a
        // boundary element (e.g. `<br>`) in between correctly drops them —
        // and the node doesn't already start with one (a cross-node run
        // ending here collapses to the one already present).
        let mut migrated_prefix_count = 0;
        if pending_in > 0
            && !text.starts_with("\u{00A0}")
            && has_inline_adjacent(doc, cascade, &parent_of, idx, -1)
        {
            migrated_prefix_count = pending_in;
            text = "\u{00A0}".repeat(pending_in as usize) + &text;
        }
        // white-space phase 1 collapsing。
        // pre 系は無変換 (tab 展開は後段)。collapse 系のみ trim 位置付きで変換。
        let start = line_start_pos(doc, cascade, &parent_of, idx);
        let trim_end = trail_trim(doc, cascade, &parent_of, idx);
        let mut text = collapse_ws(&text, cv.white_space, start, trim_end).into_owned();
        // CSS text transforms run on the post-collapse text. In particular,
        // `full-width` must see only the surviving U+0020 space; applying it
        // before whitespace collapsing would turn every source space into
        // U+3000 and incorrectly prevent the collapse.
        let mut transformed = apply_text_transform(&text, cv.text_transform, &language);
        if migrated_prefix_count > 0 && pending_full_width {
            let mut migrated = 0;
            let mut with_full_width_spaces = String::with_capacity(transformed.len());
            for c in transformed.chars() {
                if migrated < migrated_prefix_count && c == '\u{00A0}' {
                    with_full_width_spaces.push('\u{3000}');
                    migrated += 1;
                } else {
                    with_full_width_spaces.push(c);
                }
            }
            transformed = with_full_width_spaces;
        }
        // HTML tokenization may place U+0307 in its own text node. In
        // Turkic lowercase, an `I` followed by that combining dot maps to
        // `i`; fold the split pair back into the preceding shaping job.
        if transformed
            .chars()
            .eq(std::iter::once(char::from_u32(0x0307).unwrap()))
            && matches!(cv.text_transform, TextTransform::Lowercase)
            && (language_matches(&language, "tr") || language_matches(&language, "az"))
            && let Some(previous) = jobs.last_mut()
            && parent_of[previous.idx] == parent_of[idx]
            && previous.text.ends_with('ı')
        {
            previous.text.pop();
            previous.text.push('i');
            continue;
        }
        text = transformed;
        // Each DOM text node gets its own Parley layout today. Preserve the
        // joining context that CSS Text requires across adjacent inline boxes
        // by mirroring the WPT reference's zero-width joiners at the edges of
        // joining-script runs. The joiner has no painted glyph or advance.
        text = add_boundary_shaping_joiners(
            text,
            boundary_shaping_adjacent(doc, cascade, &parent_of, idx, -1),
            boundary_shaping_adjacent(doc, cascade, &parent_of, idx, 1),
        );
        // A soft hyphen is only an opportunity when hyphenation is enabled.
        // Removing it for `hyphens: none` also prevents the shaping and line
        // breaker from treating it as a discretionary break.
        if cv.hyphens == Hyphens::None {
            text.retain(|c| c != '\u{00AD}');
        }
        let family_str: String = family_str_of(cv);
        // tab-stop metrics は block-container 祖先の font (CSS Text 3 §4.2)。
        // 見つからなければ自 font (fail-safe — 既存挙動と同等)。
        let mcv = nearest_block_container(doc, cascade, &parent_of, idx)
            .map(|b| &cascade.computed[b])
            .unwrap_or(cv);
        // Page side margins define the inline containing block.  Keep the
        // shaped run on that same width so line breaks agree with the page
        // content box even when the remaining strip is very narrow.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        let multicol_advance =
            multicol_column_width_for_text(cascade, &parent_of, idx, max_advance);
        // cov:ignore: authored auto-width fallback is exercised by the ignored foundation WPT run.
        let authored_advance = if multicol_advance.is_none() {
            authored_containing_width_with_resolved_ch(doc, cascade, &parent_of, idx, max_advance)
        } else {
            None
        };
        // cov:ignore: multicol text shaping width selection is exercised by the ignored foundation WPT run.
        let shape_advance = if let Some(column_width) = multicol_advance {
            column_width
        } else if page_width > max_advance
            && (is_leading_body_text(doc, body_id, idx) || has_out_of_flow_ancestor(parent_of[idx]))
        {
            page_width
        } else if let Some(width) = authored_advance {
            width
        } else {
            max_advance
        };
        let autospace_value = if has_vertical_writing_mode(cascade, &parent_of, idx) {
            // This implementation normalizes vertical autospace to the
            // horizontal layout path until vertical inline metrics are implemented.
            TextAutospace::NoAutospace // cov:ignore: vertical-writing autospace is exercised by the ignored vertical WPT runs.
        } else {
            cv.text_autospace
        };
        // Own a cross-node boundary on the following nonempty text run.
        // Assigning it to both neighbors would double the advance.
        let autospace_before = autospace_adjacent_edge_char(doc, cascade, &parent_of, idx, -1);
        let autospace_width = if matches!(autospace_value, TextAutospace::NoAutospace) {
            0.0
        } else {
            let style = match cv.font_style {
                StyleFontStyle::Normal => 0,
                StyleFontStyle::Italic => 1,
                StyleFontStyle::Oblique => 2,
                // cov:ignore: StyleFontStyle currently has only three variants.
                _ => 0,
            };
            let key = (
                family_str.clone(),
                cv.font_size.px().to_bits(),
                cv.font_weight.to_bits(),
                style,
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = ic_probes.entry(key.clone()) {
                entry.insert(probe_ic_text_advance(
                    fonts,
                    layout_cx,
                    &family_str,
                    cv.font_size.px(),
                    cv.font_weight,
                    cv.font_style,
                ));
            }
            ic_probes.get(&key).copied().unwrap_or(0.0) * 0.125
        };
        let autospace_boxes = text_autospace_boxes_with_width(
            &text,
            autospace_value,
            &language,
            autospace_width,
            autospace_before,
            None,
        );
        let simple_pre_block = matches!(cv.white_space, WhiteSpace::Pre)
            && parent_of[idx].is_some_and(|parent| {
                matches!(
                    cascade.computed[parent].display,
                    DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::ListItem
                ) && nearest_block_container(doc, cascade, &parent_of, idx) == Some(parent)
                    && doc.nodes[parent]
                        .children
                        .iter()
                        .filter(|&&child| doc.nodes[child].is_in_document())
                        .count()
                        == 1
            });
        // A single preserved text run wrapped in inline elements should use
        // the same metric quantization as a direct simple pre block.  Ignore
        // an otherwise-empty inline sibling containing only the terminal
        // preserved newline; it does not establish another line box.
        let simple_preserved_run = matches!(cv.white_space, WhiteSpace::PreWrap)
            && !text.contains('\n')
            && has_authored_non_whitespace_text(doc, idx)
            && parent_of[idx].is_some_and(|parent| {
                let parent_display = cascade.computed[parent].display;
                (parent_display == DisplayValue::Inline || parent_display == DisplayValue::Contents)
                    && doc.nodes[parent]
                        .children
                        .iter()
                        .filter(|&&child| has_authored_non_whitespace_text(doc, child))
                        .count()
                        == 1
                    && nearest_block_container(doc, cascade, &parent_of, idx).is_some_and(|block| {
                        doc.nodes[block].children.len() > 1
                            && matches!(cascade.computed[block].display, DisplayValue::InlineBlock)
                            && doc.nodes[block].children.iter().all(|&child| {
                                child == parent
                                    || (doc.nodes[child].kind() == NodeKind::Element
                                        && is_inline_element_box(cascade, child)
                                        && doc.nodes[child].tag_name() != Some("br")
                                        && !has_authored_non_whitespace_text(doc, child))
                                    || (doc.nodes[child].kind() == NodeKind::Text
                                        && text_of(doc, child) == Some("\n"))
                            })
                    })
            });
        // A terminal preserved newline establishes no following empty line.
        // Keeping it as a standalone Parley layout would nevertheless create
        // two line metrics, and letter-spacing makes that artifact visible.
        if text == "\n" && !has_inline_adjacent(doc, cascade, &parent_of, idx, 1) {
            continue;
        }
        jobs.push(Job {
            idx,
            had_tab: text.contains('\t'),
            text,
            family_str,
            font_size_raw: cv.font_size.px(),
            font_weight_raw: cv.font_weight,
            font_style: cv.font_style,
            line_height_raw: cv.line_height,
            letter_spacing_raw: cv.letter_spacing.px(),
            letter_spacing_ch_factor: cv.letter_spacing_ch_factor,
            word_spacing_raw: cv.word_spacing.px(),
            word_spacing_ch_factor: cv.word_spacing_ch_factor,
            tab_size: cv.tab_size,
            white_space: cv.white_space,
            word_break: cv.word_break,
            line_break: cv.line_break,
            line_break_override_enabled: false,
            has_inline_root_ancestor: has_inline_root_ancestor(doc, &parent_of, idx),
            overflow_wrap: cv.overflow_wrap,
            text_wrap_mode: cv.text_wrap,
            autospace_boxes,
            // A simple `white-space: pre` block is non-wrapping; this also
            // keeps its measured tab cursor independent of later line breaks.
            nowrap: simple_pre_block
                || cv.white_space == WhiteSpace::Nowrap
                || cv.text_wrap == TextWrapMode::Nowrap,
            max_advance: shape_advance,
            metrics_family: family_str_of(mcv),
            metrics_size: mcv.font_size.px(),
            metrics_weight: mcv.font_weight,
            metrics_style: mcv.font_style,
            metrics_letter_spacing_raw: mcv.letter_spacing.px(),
            metrics_letter_spacing_ch_factor: mcv.letter_spacing_ch_factor,
            metrics_word_spacing_raw: mcv.word_spacing.px(),
            metrics_word_spacing_ch_factor: mcv.word_spacing_ch_factor,
            simple_pre_block,
            simple_preserved_run,
            tab_spacing_ranges: Vec::new(),
        });
    }

    if jobs.is_empty() {
        return;
    }

    // Resolve `ch` before measuring block-container stops or text prefixes.
    let mut ch_probes: std::collections::HashMap<(String, u32, u32, u8), f32> =
        std::collections::HashMap::new();
    for job in &jobs {
        if job.letter_spacing_ch_factor.is_some() || job.word_spacing_ch_factor.is_some() {
            let key = shape_font_key(job);
            if let std::collections::hash_map::Entry::Vacant(entry) = ch_probes.entry(key) {
                entry.insert(probe_ch_text_advance(
                    fonts,
                    layout_cx,
                    &job.family_str,
                    job.font_size_raw,
                    job.font_weight_raw,
                    job.font_style,
                ));
            }
        }
        if job.metrics_letter_spacing_ch_factor.is_some()
            || job.metrics_word_spacing_ch_factor.is_some()
        {
            let key = font_key(job);
            if let std::collections::hash_map::Entry::Vacant(entry) = ch_probes.entry(key) {
                entry.insert(probe_ch_text_advance(
                    fonts,
                    layout_cx,
                    &job.metrics_family,
                    job.metrics_size,
                    job.metrics_weight,
                    job.metrics_style,
                ));
            }
        }
    }
    for job in &mut jobs {
        if let Some(factor) = job.letter_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&shape_font_key(job))
        {
            job.letter_spacing_raw = factor * advance;
        }
        if let Some(factor) = job.word_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&shape_font_key(job))
        {
            job.word_spacing_raw = factor * advance;
        }
        if let Some(factor) = job.metrics_letter_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&font_key(job))
        {
            job.metrics_letter_spacing_raw = factor * advance;
        }
        if let Some(factor) = job.metrics_word_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&font_key(job))
        {
            job.metrics_word_spacing_raw = factor * advance;
        }
        if !matches!(
            job.white_space,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
        ) || !job.text.contains('\t')
        {
            continue;
        }
        if !job.simple_pre_block {
            // Inline siblings need one shared post-layout cursor. Keep their
            // established expansion until the inline bridge exposes that state.
            let space_advance = probe_text_advance(
                fonts,
                layout_cx,
                " ",
                &job.metrics_family,
                job.metrics_size,
                job.metrics_weight,
                job.metrics_style,
            );
            let (text, tab_lens) = expand_tabs_locally(&job.text, job.tab_size, space_advance);
            remap_boxes_through_tab_rewrite(&job.text, &tab_lens, &mut job.autospace_boxes);
            job.text = text;
            continue;
        }
        let block_space_advance = probe_text_full_width(
            fonts,
            layout_cx,
            " ",
            TextProbeStyle {
                family_str: &job.metrics_family,
                font_size_px: job.metrics_size,
                font_weight: job.metrics_weight,
                font_style: job.metrics_style,
                letter_spacing: job.metrics_letter_spacing_raw,
                word_spacing: job.metrics_word_spacing_raw,
            },
        );
        let interval = tab_stop_advance(job.tab_size, block_space_advance);
        let space_base_advance = probe_text_full_width(
            fonts,
            layout_cx,
            " ",
            TextProbeStyle {
                family_str: &job.family_str,
                font_size_px: job.font_size_raw,
                font_weight: job.font_weight_raw,
                font_style: job.font_style,
                letter_spacing: job.letter_spacing_raw,
                word_spacing: 0.0,
            },
        );
        let text_word_spacing = if job.word_spacing_ch_factor.is_some() {
            job.word_spacing_raw
        } else {
            0.0
        };
        let (text, spacing_ranges, tab_lens) =
            replace_tabs_with_styled_spaces(&job.text, interval, space_base_advance, |segment| {
                probe_text_full_width(
                    fonts,
                    layout_cx,
                    segment,
                    TextProbeStyle {
                        family_str: &job.family_str,
                        font_size_px: job.font_size_raw,
                        font_weight: job.font_weight_raw,
                        font_style: job.font_style,
                        letter_spacing: job.letter_spacing_raw,
                        word_spacing: text_word_spacing,
                    },
                )
            });
        remap_boxes_through_tab_rewrite(&job.text, &tab_lens, &mut job.autospace_boxes);
        job.text = text;
        job.tab_spacing_ranges = spacing_ranges;
    }

    for job in &mut jobs {
        let adapter_enabled = should_enable_nbsp_adapter(
            job.line_break,
            job.white_space,
            job.nowrap,
            &job.text,
            job.had_tab,
            job.has_inline_root_ancestor,
        );
        job.line_break_override_enabled = should_enable_anywhere_callback(
            job.line_break,
            job.white_space,
            job.nowrap,
            &job.text,
            adapter_enabled,
        );
        if job.white_space == WhiteSpace::PreWrap && !adapter_enabled {
            job.line_break_override_enabled = false;
        }
        if !adapter_enabled {
            continue;
        }

        let word_spacing = if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
            job.word_spacing_raw
        } else {
            0.0
        };
        let advance = probe_text_full_width(
            fonts,
            layout_cx,
            "\u{00A0}",
            TextProbeStyle {
                family_str: &job.family_str,
                font_size_px: job.font_size_raw,
                font_weight: job.font_weight_raw,
                font_style: job.font_style,
                letter_spacing: job.letter_spacing_raw,
                word_spacing,
            },
        );
        if !advance.is_finite() || advance <= 0.0 {
            job.line_break_override_enabled = false;
            continue;
        }
        match replace_nbsp_with_inline_boxes(&job.text, advance, &job.autospace_boxes) {
            Some((text, inline_boxes)) => {
                job.text = text;
                job.autospace_boxes = inline_boxes;
            }
            None => job.line_break_override_enabled = false,
        }
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
            let letter_spacing = sanitize_finite(
                job.letter_spacing_raw,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "letter-spacing",
                &mut warnings,
            );
            let word_spacing = sanitize_finite(
                job.word_spacing_raw,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "word-spacing",
                &mut warnings,
            );
            let font_family = FontFamily::from(job.family_str.as_str());
            // In this simple block run, Taffy's fractional line flow must use
            // the same metrics as Parley's painted baselines.
            let quantize_metrics = !job.simple_pre_block && !job.simple_preserved_run;
            let mut builder = layout_cx.ranged_builder(fonts, &job.text, 1.0, quantize_metrics);
            builder.set_line_break_override(parley_line_break_override(
                job.line_break_override_enabled,
            ));
            builder.push_default(StyleProperty::FontFamily(font_family));
            builder.push_default(StyleProperty::FontSize(font_size_px));
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
            builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                job.font_style,
            )));
            builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                line_height,
            )));
            builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
            // Apply negative CSS word-spacing lengths in addition to the existing
            // font-metric-aware `ch` path. Other non-`ch` values remain deferred.
            if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
                builder.push_default(StyleProperty::WordSpacing(word_spacing));
            }
            for (range, tab_word_spacing) in &job.tab_spacing_ranges {
                builder.push(StyleProperty::WordSpacing(*tab_word_spacing), range.clone());
            }
            builder.push_default(StyleProperty::WordBreak(parley_word_break(
                job.word_break,
                job.line_break,
            )));
            builder.push_default(StyleProperty::OverflowWrap(parley_overflow_wrap(
                job.word_break,
                job.line_break,
                job.overflow_wrap,
            )));
            builder.push_default(StyleProperty::TextWrapMode(parley_text_wrap_mode(
                job.text_wrap_mode,
                job.nowrap,
            )));
            for inline_box in &job.autospace_boxes {
                builder.push_inline_box(inline_box.clone());
            }
            let mut layout: Layout<()> = builder.build(&job.text);
            layout.break_all_lines(if job.nowrap {
                None
            } else {
                Some(job.max_advance)
            });
            layout.align(Alignment::Start, AlignmentOptions::default());
            doc.layout_warnings.extend(warnings);
            if let Some(t) = doc.nodes[job.idx].data.as_text_mut() {
                t.text_layout = Some(layout);
                t.snap_glyph_x_to_1_64 = job.simple_pre_block || job.simple_preserved_run;
            }
        }
        return;
    }

    let base_fonts: FontContext = fonts.clone();
    // Parallel shaping: chunked to amortize FontContext/LayoutContext setup.
    // Per-job `LayoutContext::new()` is expensive (ICU AnalysisDataSources etc.)
    // so we reuse one FontContext+LayoutContext per rayon chunk.
    // Chunk size 128 reduces clones to ~4/16 for 500/2000 jobs.
    let results: Vec<(usize, Layout<()>, Vec<LayoutWarn>, bool)> = jobs
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
                let letter_spacing = sanitize_finite(
                    job.letter_spacing_raw,
                    -MAX_TAFFY_MAGNITUDE,
                    MAX_TAFFY_MAGNITUDE,
                    "letter-spacing",
                    &mut warnings,
                );
                let word_spacing = sanitize_finite(
                    job.word_spacing_raw,
                    -MAX_TAFFY_MAGNITUDE,
                    MAX_TAFFY_MAGNITUDE,
                    "word-spacing",
                    &mut warnings,
                );
                let font_family = FontFamily::from(job.family_str.as_str());
                let quantize_metrics = !job.simple_pre_block && !job.simple_preserved_run;
                let mut builder =
                    lcx.ranged_builder(&mut fonts_thread, &job.text, 1.0, quantize_metrics);
                builder.set_line_break_override(parley_line_break_override(
                    job.line_break_override_enabled,
                ));
                builder.push_default(StyleProperty::FontFamily(font_family));
                builder.push_default(StyleProperty::FontSize(font_size_px));
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
                builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                    job.font_style,
                )));
                builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                    line_height,
                )));
                builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
                if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
                    builder.push_default(StyleProperty::WordSpacing(word_spacing));
                }
                for (range, tab_word_spacing) in &job.tab_spacing_ranges {
                    builder.push(StyleProperty::WordSpacing(*tab_word_spacing), range.clone());
                }
                builder.push_default(StyleProperty::WordBreak(parley_word_break(
                    job.word_break,
                    job.line_break,
                )));
                builder.push_default(StyleProperty::OverflowWrap(parley_overflow_wrap(
                    job.word_break,
                    job.line_break,
                    job.overflow_wrap,
                )));
                builder.push_default(StyleProperty::TextWrapMode(parley_text_wrap_mode(
                    job.text_wrap_mode,
                    job.nowrap,
                )));
                // cov:ignore: the parallel preshape path is exercised by resource-backed WPT runs with large inline job sets.
                for inline_box in &job.autospace_boxes {
                    builder.push_inline_box(inline_box.clone());
                }
                let mut layout: Layout<()> = builder.build(&job.text);
                layout.break_all_lines(if job.nowrap {
                    None
                } else {
                    Some(job.max_advance)
                });
                layout.align(Alignment::Start, AlignmentOptions::default());
                out.push((
                    job.idx,
                    layout,
                    warnings,
                    job.simple_pre_block || job.simple_preserved_run,
                ));
            }
            out
        })
        .collect();

    for (idx, layout, warnings, snap_glyph_x_to_1_64) in results {
        doc.layout_warnings.extend(warnings);
        if let Some(t) = doc.nodes[idx].data.as_text_mut() {
            t.text_layout = Some(layout);
            t.snap_glyph_x_to_1_64 = snap_glyph_x_to_1_64;
        }
    }
}

#[cfg(test)]
mod tests;
