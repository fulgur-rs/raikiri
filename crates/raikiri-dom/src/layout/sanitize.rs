use super::*;

// ---------------------------------------------------------------------------
// 非有限 f32 の guard
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
/// これらは絶対化を cascade に入れるまで、`layout.rs` の
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
/// # 入力側 bound の射程と、出力側 guard による決着
///
/// 本定数は [`sanitize_taffy`] 経由で **px 幾何と percentage の fraction の
/// 両方**に適用されている。px 側については上の #4552 の導出がそのまま効くが、
/// **fraction 側の bound としては、値をどれだけ小さく取っても不十分である。**
///
/// percentage の containing block に対する解決は used value 層 (taffy 側) で
/// 起き、**nest するたびに再び掛かる**ので深さについて指数的に複利する。A4
/// (793.7 px) を起点にすると f32 が非有限になるまでの余裕は約 35.6 桁なので、
/// fraction の上限を `F` (> 1) としたとき最初に非有限になる深さは概ね
/// `35.6 / log10(F)` — **常に有限**である。修正前の depth range 実測はこの
/// model と一致する:
///
/// | decl | fraction | `35.6 / log10(F)` | 実測の最初の非有限 depth |
/// |---|---|---|---|
/// | `width: 1e9%` (本定数ちょうど) | 1e7 | 5.1 | 6 |
/// | `width: 100000%` | 1e3 | 11.9 | 12 |
/// | `width: 10000%` | 1e2 | 17.8 | 18 |
/// | `width: 1000%` | 1e1 | 35.6 | 36 |
///
/// (`padding-left` を同じ値にすると test setup で 4 / 8 / — / 25 とより
/// 浅い。padding は `location` / `scrollable_overflow_rect` の累積にも寄与するため。)
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
/// ±Inf / NaN を taffy の内部演算に入れない役割が残っており (site 1-4 の
/// test がこれを check している)、出力側 clamp は「arena に
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
pub(crate) const MAX_TAFFY_MAGNITUDE: f32 = 1e7;

/// parley に渡す `font-size` の上限 (px)。
///
/// taffy 幾何 ([`MAX_TAFFY_MAGNITUDE`]) と分けているのは CSS Values 4 §5 の
/// 「supported range は property / context ごとに違ってよい」に従うため
/// (site ごとに target context が違うので一律にしない方針)。font-size の
/// target context は parley → skrifa の glyph scaler
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
/// 下流 sink の帰結は当初 plausible なリスクとして未 characterize のままだったが、
/// 本 guard の実装時に実測した:
/// `sanitize_finite` を恒等関数に差し替えて
/// `nonfinite_font_size_is_clamped_before_parley` を単独実行すると
/// **25 秒経っても終了しない**。すなわち非有限 font-size は parley の shaping を
/// 有界時間で終わらせない。
///
/// 対して site 1-4 の taffy 側 test は **本 test 入力では**即座に assert 失敗する
/// (値が壊れるだけ)。これは「taffy は非有限で hang しない」という一般命題では
/// ない — 測ったのは 5 本の入力だけである。taffy 内部の used value に対する
/// 挙動は下流 sink 側の characterize 課題として別途残る。
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
pub(crate) const MAX_FONT_SIZE_PX: f32 = 1e6;

/// parley に渡す `line-height` の unitless multiplier
/// (`ComputedLineHeight::Number` → `parley::LineHeight::FontSizeRelative`)
/// の上限。
///
/// # 値の決定 (1e6、実測による overflow 回避)
///
/// parley は `FontSizeRelative(value) * font_size` を計算する
/// (`parley-0.10.0/src/layout/data.rs` の `push_run` 内 line height 計算)。
/// `font_size` はここに渡る時点で [`MAX_FONT_SIZE_PX`] (`1e6`) 以下に
/// clamp 済みなので、`value` 側も同じ `1e6` に抑えれば積は高々 `1e12` —
/// `f32::MAX` (`≈3.4e38`) から 26 桁以上の余裕があり、finite × finite の
/// 乗算で桁あふれして `+Inf` になることはない。
///
/// この余裕が必要な理由は実測済: `value = f32::MAX` を素通しすると
/// `f32::MAX * font_size` (`font_size` が `1.0` を超える限り) が overflow して
/// `+Inf` になり、`+Inf` な line height は `sanitize_line_height` の doc が
/// 挙げる hang 経路に入る。`1e6` という具体的な数値自体に他の根拠はなく、
/// 「桁あふれしないことが確認できる finite な上限」であれば足りる —
/// [`MAX_FONT_SIZE_PX`] と同じ値を採ったのは、typographic に意味のある
/// line-height multiplier (実用上せいぜい 1 桁台) から見て両方とも同程度に
/// 過大な安全域だから。
///
/// # 結合の compile-time check
///
/// 上記の overflow 非発生の論証は「両定数が同じ `1e6`」という結合そのものに
/// 依存しており、どちらか一方だけを書き換えると崩れる。直下の
/// `const _: () = assert!(...)` は「積は高々 `1e12`」という上記 paragraph
/// 自体の関係式を compile time に固定する — `f32::MAX` 直下ではなく現在の
/// 積そのものを band として check してあるので、積が**増える**方向にどちらか
/// の定数を変更すればビルドが落ちる (減る方向は安全域が広がるだけなので
/// 素通しする)。値だけ緩めて通すのではなく、両定数と overflow 論証を
/// 併せて見直すこと。[`MAX_FONT_SIZE_PX`] 自身の妥当域は同じ形の check を
/// `clamp_limits_are_in_the_documented_range` (test) が別途固定している。
pub(crate) const MAX_LINE_HEIGHT_NUMBER: f32 = 1e6;

// `f64` で積を取るのは、両定数を `f32` へ丸めた積が `1e12` 境界の
// どちら側に丸まるかという 1 ULP 未満の差にこの検査を左右させないため
// (`f32` 同士の積が overflow しても trap せず `+Inf` に飽和するだけで、
// `+Inf <= 1e12` は正しく false と評価される。ここでの懸念は overflow
// ではなく丸め境界の精度)。
const _: () = assert!((MAX_LINE_HEIGHT_NUMBER as f64) * (MAX_FONT_SIZE_PX as f64) <= 1e12);

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
/// panic しない (`LayoutError` も返さない) — **clamp して続行**する方針である。
///
/// # なぜ silent clamp ではないのか
///
/// `log` / `tracing` は workspace に依存が無い (`grep` → 0 hit) が、**それが
/// 理由ではない** — 同一 crate の `fonts.rs` に dep 追加ゼロの診断機構が既に
/// ある (`FontWarn` enum + `FontWarnObserver = Option<&mut dyn FnMut(&FontWarn)>`
/// + `emit_warn`)。
///
/// 当初この observer を本 site まで通すと公開
/// signature に波及すると判断し silent のままにしていた: `sanitize_*` は
/// `bridge_*` → `apply_computed_to_style` → `layout_single_page` の奥にあり、
/// また出力側の choke point (`sanitize_taffy_layout`) は
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout` から
/// 呼ばれる — これは `taffy` crate 側が固定した trait method signature なので
/// **観測用引数を追加できない**。
///
/// [`crate::diag::emit_warn_via`] 共通機構を導入し、
/// この 2 点を以下で解決した:
/// - `bridge_*` → `apply_computed_to_style` の chain は crate 内 private
///   function のみで構成されるため、`diag: &mut Vec<LayoutWarn>` を通すのは
///   crate-internal な signature 変更で完結する (pub シグネチャは無傷)。
/// - `set_unrounded_layout` は `self` (`&mut Document`) は受け取れるので、
///   observer を **closure ではなく owned buffer**
///   (`Document::layout_warnings`) として `self` に持たせることで、
///   trait signature を変えずに choke point からも push できるようにした。
///   `layout_single_page` がこの buffer をパスの最後で drain し、
///   `fonts.rs` と同じ `emit_warn_via` 経由で observer-or-eprintln に流す。
///
/// 「per-node で裸の `eprintln!` を撒いて spam する」ことは避けている —
/// [`push_layout_warn`] は実際に clamp が起きた (値が変わった) 場合のみ
/// event を積むので、通常範囲の layout は buffer に何も残らない。これは
/// `FontWarn` が「warn+skip の異常」だけを observer に渡し、処理した file
/// 全部を都度報告しないのと同じ設計原則である。
///
/// **残余リスク (部分的にのみ縮小)**:
/// 将来 absolutize 側に本物の算術 bug (例: 単位換算ミスで `1e9px`) が入ると、
/// 本 guard が 1e7 に吸収して**「それらしい layout」として描画されてしまう**
/// リスクは元々あった — NaN や破綻として可視化されない。これはかつて
/// 削除した fail-quiet arm (`Em(_) => length(0.0)`) と**同じ class の
/// 残余リスク**である。
///
/// 今は clamp が発生するたび [`LayoutWarn::NonFiniteClamped`] が
/// observer-or-eprintln 経由で外に出るが、**「解消」ではなく「silent から
/// stderr-visible への降格」**と正確に言うべきである — `layout_single_page`
/// に external observer を差し込む口は現状無い (`LayoutWarnObserver`
/// scaffolding の doc参照) ので、本 crate 内に stderr を能動的に監視する
/// consumer が無い限り、この event は誰にも読まれない。`fonts.rs` の
/// `FontWarn` も同じ状態 (observer 無しなら stderr のみ) なので同水準の
/// 可視性にはなったが、「値の病理が可視化される」と言えるのは stderr を
/// 見ている human operator がいる場合に限る。
pub(crate) fn sanitize_finite(
    v: f32,
    min: f32,
    max: f32,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> f32 {
    let clamped = if v.is_nan() { 0.0 } else { v.clamp(min, max) };
    // `v != clamped` は NaN 入力でも正しく true になる (NaN の比較は IEEE 754
    // で常に false 「以外」= `!=` は true) ので、NaN → 0.0 の代入も
    // out-of-range 値の clamp も同じ条件で拾える。範囲内の通常値は
    // `clamped == v` なので何も積まない (spam 回避、上の doc 参照)。
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site,
                raw: v,
                clamped,
            },
        );
    }
    clamped
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
///
/// `site` は [`LayoutWarn::NonFiniteClamped`] の call-site label としてのみ
/// 使う (clamp の算術には影響しない)。
pub(crate) fn sanitize_taffy(v: f32, site: &'static str, diag: &mut Vec<LayoutWarn>) -> f32 {
    sanitize_finite(v, -MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE, site, diag)
}

/// Structured warn event for this module's non-finite-clamp diagnostic sites
/// (`sanitize_finite` / `sanitize_taffy` / `sanitize_taffy_layout` /
/// `sanitize_font_weight`). Sibling
/// of [`crate::fonts::FontWarn`], generalized via the
/// shared [`crate::diag::emit_warn_via`] mechanism so
/// the "silent clamp" residual risk documented on [`sanitize_finite`] gets
/// the same observability `fonts.rs` already has.
///
/// Every variant is fully owned (no borrowed `Path`, unlike `FontWarn`)
/// because these clamp sites only ever see primitive `f32` values. See
/// [`crate::diag`]'s module doc for why this owned shape — not `FontWarn`'s
/// borrowed one — is what a shared generic `Observer<W>` type could actually
/// have supported, and why a macro was used instead so both shapes share one
/// mechanism anyway.
///
/// No `#[non_exhaustive]` (unlike `FontWarn`, which is `pub`): that attribute
/// only constrains *downstream crates*, and this enum is `pub(crate)` with no
/// external consumer to protect. Add it back if this type is ever promoted
/// to a public export.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LayoutWarn {
    /// A non-finite (NaN / +-Inf) or out-of-range `f32` was clamped to a
    /// finite in-range value before being handed to one of this module's
    /// sink boundaries: `taffy::Style` or the `Node.unrounded_layout` arena
    /// field (sites 1-5, `sanitize_finite` / `sanitize_taffy` /
    /// `sanitize_taffy_layout`),
    /// `parley::FontWeight::new` (site 6, `sanitize_font_weight`), or
    /// `StyleProperty::LineHeight`'s two numeric sub-values (sites 7-8,
    /// `sanitize_line_height`). Only
    /// emitted when clamping actually changed the
    /// value (not on every call) so ordinary in-range layouts stay silent —
    /// the "warn+skip" shape `FontWarn` uses, not a per-node trace.
    NonFiniteClamped {
        /// Call-site label (e.g. `"font-size"`, `"margin"`,
        /// `"layout.size"`) — a human-readable category, not a stable
        /// machine-parseable identifier.
        site: &'static str,
        /// Pre-clamp value (may be NaN or +-Inf).
        raw: f32,
        /// Post-clamp value actually used.
        clamped: f32,
    },
    /// `suppressed` additional [`LayoutWarn::NonFiniteClamped`] events were
    /// dropped once [`LAYOUT_WARN_CAP`] was reached during a single
    /// `layout_single_page` pass, bounding memory / `eprintln!` spam under a
    /// pathological input that clamps every field of every node (e.g. deep
    /// `width: 200%` nesting — see [`MAX_TAFFY_MAGNITUDE`]'s doc). Emitted at
    /// most once per pass, after all the real events it summarizes.
    Truncated {
        /// Count of additional `NonFiniteClamped` events dropped after the
        /// cap was reached.
        suppressed: usize,
    },
    /// One or more subtrees had a broken **parent/child geometry invariant**
    /// and were reset to a deterministic zero
    /// [`taffy::Layout`] by [`enforce_layout_invariants`]. This is a
    /// different failure class than [`LayoutWarn::NonFiniteClamped`]: that
    /// variant fires when a single `f32` field was out of range, this one
    /// fires when every individual field of the (already per-field-clamped)
    /// `Layout`s involved was in range, but the *relationship* between two
    /// or more fields — possibly on different nodes — was not (e.g. a
    /// node's own content box went negative, or a child's border box did
    /// not fit inside its parent's once one of the two had its actual value
    /// approximated by [`sanitize_taffy_layout`]). See
    /// [`enforce_layout_invariants`]'s doc for exactly which two invariants
    /// are checked and why unconditionally checking cross-node containment
    /// would be spec-incorrect.
    ///
    /// Aggregated per invariant (at most one event per invariant kind per
    /// `layout_single_page` pass, each counting every subtree it reset)
    /// rather than one event per reset subtree, so a pathological input
    /// that trips the same invariant on many nodes cannot reintroduce the
    /// per-node spam [`LAYOUT_WARN_CAP`] exists to bound.
    GeometryInvariantViolated {
        /// Which invariant was violated — `"content_box_non_negative"` or
        /// `"child_within_parent_border_box"` (see
        /// [`enforce_layout_invariants`]'s doc). Like `NonFiniteClamped`'s
        /// `site`, a human-readable category, not a stable
        /// machine-parseable identifier.
        ///
        /// The `"child_within_parent_border_box"`
        /// value can no longer actually occur — [`child_within_parent_border_box`]
        /// (the predicate) now always returns `true`, so
        /// [`enforce_layout_invariants`]'s containment branch that would
        /// produce this event is unreachable for any input. It remains
        /// listed here (and the branch remains in the code) because whether
        /// to remove the dead invariant check entirely is a separate,
        /// explicitly deferred decision. Do not treat this
        /// value's continued presence in this doc as evidence the check is
        /// still live.
        invariant: &'static str,
        /// Number of distinct subtree roots this invariant caused
        /// [`enforce_layout_invariants`] to reset in this pass (not a count
        /// of individual arena nodes touched — a reset subtree may contain
        /// further descendants that also got zeroed as part of the same
        /// reset).
        subtree_count: usize,
    },
}

impl std::fmt::Display for LayoutWarn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutWarn::NonFiniteClamped { site, raw, clamped } => write!(
                f,
                "{site}: clamped non-finite/out-of-range value {raw} to {clamped}"
            ),
            LayoutWarn::Truncated { suppressed } => write!(
                f,
                "{suppressed} additional layout clamp warning(s) suppressed (buffer cap reached)"
            ),
            LayoutWarn::GeometryInvariantViolated {
                invariant,
                subtree_count,
            } => write!(
                f,
                "{invariant}: {subtree_count} subtree(s) had a broken parent/child geometry invariant and were reset to a zero layout"
            ),
        }
    }
}

/// Observer alias for [`LayoutWarn`] — sibling of `fonts.rs`'s
/// `FontWarnObserver`. Unlike that alias, this one carries no inner lifetime
/// (`LayoutWarn` is fully owned), so it is a plain, non-higher-ranked
/// `Option<&mut dyn FnMut(&LayoutWarn)>`. Not yet reachable from any public
/// entry point — see [`Document::layout_warnings`](crate::document::Document)
/// for why (dom→paint wall: `layout_single_page`'s signature is consumed by
/// `raikiri-paint` and the `raikiri` crate, so adding a parameter — or a new
/// `_with_observer` sibling — to it is a decision for that wall, not this
/// task). This scaffolding exists so a future `_with_observer` addition only
/// has to plumb one new parameter through, rather than re-deriving the whole
/// mechanism.
pub(crate) type LayoutWarnObserver<'o> = Option<&'o mut dyn FnMut(&LayoutWarn)>;

/// Emit a [`LayoutWarn`] event: call the observer if `Some`, otherwise
/// `eprintln!` (matches [`crate::fonts`] の `emit_warn`'s shape exactly, via the
/// shared [`crate::diag::emit_warn_via`] macro).
pub(crate) fn emit_layout_warn(observer: &mut LayoutWarnObserver<'_>, event: LayoutWarn) {
    crate::diag::emit_warn_via!(observer, "[raikiri-dom::layout]", event);
}

/// Cap on buffered [`LayoutWarn::NonFiniteClamped`] events per
/// `layout_single_page` pass.
///
/// A single pathological input (e.g. deep `width: 200%` nesting hitting
/// every node, [`MAX_TAFFY_MAGNITUDE`]'s doc) can clamp every `f32` field of
/// every node in the arena, which would otherwise make both the buffer and
/// the eventual `eprintln!` replay unbounded — precisely the "per-node spam"
/// concern [`sanitize_finite`]'s doc raised about threading an observer down
/// this chain in the first place. [`push_layout_warn`] collapses anything
/// past this cap into a single running [`LayoutWarn::Truncated`] counter
/// instead of dropping it silently.
pub(crate) const LAYOUT_WARN_CAP: usize = 63;

/// Push a [`LayoutWarn`] onto `diag`, respecting [`LAYOUT_WARN_CAP`].
///
/// Once the cap is reached, further events collapse into (rather than grow)
/// a single trailing [`LayoutWarn::Truncated`] counter, so the buffer size is
/// bounded (`LAYOUT_WARN_CAP + 1`) regardless of how many clamp sites fire in
/// one pass.
pub(crate) fn push_layout_warn(diag: &mut Vec<LayoutWarn>, event: LayoutWarn) {
    if diag.len() < LAYOUT_WARN_CAP {
        diag.push(event);
        return;
    }
    match diag.last_mut() {
        Some(LayoutWarn::Truncated { suppressed }) => *suppressed += 1,
        _ => diag.push(LayoutWarn::Truncated { suppressed: 1 }),
    }
}

/// taffy が resolve した [`taffy::Layout`] の全 f32 field を
/// [`sanitize_taffy`] に通す **出力側** guard。
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
/// 非有限を含まない」は構造的な invariant であり、後付けの一括 range のように
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
/// 現在 `padding` / `border` / `scrollable_overflow_rect` / `scrollbar_size` を読む
/// consumer は無い (grep 実測) が、将来 paint がこれらを使うときは
/// 非負性を仮定しないこと。
///
/// `diag` collects [`LayoutWarn::NonFiniteClamped`] events for whichever
/// fields actually get clamped (site labels: `"layout.location"`,
/// `"layout.size"`, `"layout.scrollable_overflow_rect"`, `"layout.scrollbar_size"`,
/// `"layout.border"`, `"layout.padding"`, `"layout.margin"`). The sole caller
/// (`<Document as taffy::LayoutPartialTree>::set_unrounded_layout` in
/// `taffy_impl.rs`) passes `&mut self.layout_warnings` — an owned buffer on
/// `Document`, not a live observer — because that trait method's signature
/// is fixed by `taffy` and cannot receive one (see
/// `Document::layout_warnings`'s doc for why).
pub(crate) fn sanitize_taffy_layout(
    layout: &TaffyLayout,
    diag: &mut Vec<LayoutWarn>,
) -> TaffyLayout {
    fn size(s: Size<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Size<f32> {
        Size {
            width: sanitize_taffy(s.width, site, diag),
            height: sanitize_taffy(s.height, site, diag),
        }
    }
    fn rect(r: Rect<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Rect<f32> {
        Rect {
            left: sanitize_taffy(r.left, site, diag),
            right: sanitize_taffy(r.right, site, diag),
            top: sanitize_taffy(r.top, site, diag),
            bottom: sanitize_taffy(r.bottom, site, diag),
        }
    }
    TaffyLayout {
        order: layout.order,
        location: Point {
            x: sanitize_taffy(layout.location.x, "layout.location", diag),
            y: sanitize_taffy(layout.location.y, "layout.location", diag),
        },
        size: size(layout.size, "layout.size", diag),
        scrollable_overflow_rect: rect(
            layout.scrollable_overflow_rect,
            "layout.scrollable_overflow_rect",
            diag,
        ),
        scrollbar_size: size(layout.scrollbar_size, "layout.scrollbar_size", diag),
        border: rect(layout.border, "layout.border", diag),
        padding: rect(layout.padding, "layout.padding", diag),
        margin: rect(layout.margin, "layout.margin", diag),
    }
}

/// [`sanitize_taffy_layout`] が保証する **finiteness** の一段上のレイヤー —
/// 親子 geometry の**意味的** invariant を検査し、破れている subtree を
/// 決定的な既定 geometry (ゼロ) に置き換える。
///
/// # なぜ `sanitize_taffy_layout` だけでは閉じないか
///
/// `sanitize_taffy_layout` の doc が明言する通り、
/// その guard は **field ごとに独立に** clamp するため、box model の包含
/// 関係 (CSS Box 3 の content ⊆ padding ⊆ border) は保存しない —
/// `size.width` と `padding.{left,right}` が同時に飽和すると
/// `content_box_width()` が負になりうる。また taffy 内部の演算 chain は `LayoutOutput`
/// 経由で **clamp 前の生値** を子から親へ返す (`taffy-0.12.1` の
/// `compute/block.rs:947,973,981,1072`、`set_unrounded_layout` が呼ばれる
/// のはその**後**であり、かつ子の `Layout` を書くのは子自身ではなく
/// **親の algorithm**) ため、ある node の位置が「別の (クランプ済) node」を
/// 基準に計算されていても、各 node は「自分の field が有限」であることしか
/// 保証されない。本関数はその 2 つの隙間 — 単一 node 内の box model 包含
/// 関係、および親子間の位置関係 — を埋める。
///
/// # 検査する 2 つの invariant
///
/// 1. **content box 非負** — `Layout::content_box_width()` /
///    `content_box_height()` が両方 `>= 0.0`。**全 node に無条件で**適用する。
///    実装時に実測した (probe: `<div style="width: 10px; padding: 50px;
///    box-sizing: border-box;">` を通常経路 (`layout_single_page`) で
///    layout): taffy 自身が border box を `size.width == padding_left +
///    padding_right` (= `100.0`) まで自動的に stretch し、content box を
///    ちょうど `0.0` に floor する (`taffy-0.12.1/src/compute/mod.rs` の
///    `maybe_max(padding_border_size)` と同型の内部ロジック)。すなわち
///    **非有限 clamp が絡まない通常の CSS では content box が負になること
///    はない** — この invariant を無条件で検査しても legitimate な layout
///    を誤検出しない。
/// 2. **child の border box の原点 (`location`) が parent の border box に
///    収まる** — もともとは containment を検査する invariant として設計
///    されたが、**飽和した axis は符号を
///    問わず無条件に ok とする変更が入った** (詳細は後述の符号別の節) — つまり本 invariant は
///    もはや実際には何も検査しない (taffy の座標系は「parent border box
///    原点からの相対位置」、`taffy-0.12.1/src/tree/layout.rs` の
///    `Layout::content_box_x/y` の doc参照)。**`child.size` は見ない** —
///    本 invariant はもともと「child の **location** が parent の
///    border box 内」とだけ規定しており、child 自身の大きさは対象にしていない
///    ([`child_within_parent_border_box`] の doc「`child.size` を見ない理由」
///    節、実装時に extent (`location + size`) 版で `width: 200%` の
///    legitimate nest を誤検出することが判明した経緯を記録している)。
///
///    **以下はこの変更が入る前の設計とその根拠の記録** — 上述のとおりこの変更以降、
///    本 invariant は実際には何も検査しない無条件 accept になっている。
///    なぜ元々こちらを無条件検査にしなかったか: CSS は
///    子が親の border box をはみ出すことを普通に許す (`overflow: visible`
///    が initial 値、負 margin、固定して小さい container + 大きい content、
///    `width: 200%` のような「子が親より意図的に大きい」宣言) — raikiri は
///    今 `position: absolute` を未実装だが、それだけで十分再現する。実装時に
///    実測した (probe: parent `<div style="width: 50px; height: 50px;">` の
///    子に `<div style="width: 200px; height: 200px; margin-left:
///    -30px;">`) では parent `size=(50,50)` に対し child `location=(-30,0)`
///    — 原点自体が parent の左端 (`x=0`) より外に出ているが、これは**正しい
///    layout であって bug ではない**。無条件で検査すると legitimate な
///    layout を誤って fallback してしまうため、`child.location.x` /
///    `child.location.y` の**その axis 自身**が [`MAX_TAFFY_MAGNITUDE`] の
///    飽和境界にちょうど達している場合**だけ**、その axis を検査する
///    ([`child_within_parent_border_box`] の実装参照 — `parent.size` や
///    `child.size` が飽和しているかどうかはこの gate に関与しない)。
///    飽和が起きたということは、その field の「actual value」(近似後の値)
///    が taffy 内部の「used value」(実際の計算結果) と乖離している —
///    以前の版ではこれを根拠に「だからこそ改めて明示的に整合性を
///    検査し、破れていれば決定的な値に倒す」という設計だった。**その後
///    この設計は覆った**: 「actual value が近似
///    されている」こと自体は仕様上許容された範囲内の動作であり、収まって
///    いようといまいと本関数が reset の理由にすることはない、という結論に
///    符号を問わず統一された (詳細は後述の符号別の節)。
///    `saturated_but_contained_layout_is_not_reset` は元々「gate かつ
///    containment 違反」という conjunction を check する目的の test だった
///    が、この変更以降は assert 自体は変わらず通る (この test の fixture が
///    たまたま「収まっている」ケースなので) ものの、conjunction の主張は
///    もう成立しない — 同 test の doc および対の regression check
///    (`saturated_child_outside_parent_is_not_reset`、
///    「明らかに収まっていない」fixture でも reset されないことを直接示す
///    ために追加/改名) を参照。
///
///    **実装時の実測 (`width: 200%` を 45 段 nest、単一 chain)**: 当初は
///    extent (`location + size <= parent.size`) を検査していたが、この
///    legitimate な (どの深さでも「child は parent の 2 倍」という一貫した
///    関係を表す) declaration が深い段で誤って reset されることが判明した —
///    child の origin は常に `(0, 0)` のままなので、origin だけを見る現行の
///    定義では reset されない
///    (`nested_percentage_wide_child_chain_is_not_reset` が pin)。
///
///    **過去に発見された gate 不備 (修正済み)**: origin
///    だけを見るようにした直後の版は、なお gate を「`parent.size` /
///    `child.size` / `child.location` のいずれか 1 つでも飽和していれば
///    axis 区別なく両 axis を検査する」という条件にしていた。この形では
///    **child 自身の location が飽和していなくても** — 例えば同じ subtree の
///    どこか別の node の `parent.size` が (無関係な原因で) 飽和していた
///    だけで — legitimate な負 margin (`location.x` が通常範囲の負値、
///    例: `-30`) を持つ child が `>= 0.0` に落ちて誤って reset されうる、と
///    非軽微な finding として指摘された。現在の
///    axis 単位 gate (`child.location.x` / `.y` 自身が飽和している場合だけ、
///    その axis だけを検査する) はこれを構造的に閉じる — 検査対象になる
///    field は必ず「それ自身が近似された」field に限られるため、legitimate
///    な小さい負値がこの gate を通ることはない
///    (`saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    が直接 check する — 「`y` 軸だけでも reset の説明がつく」fixture では
///    新旧実装を区別できないという指摘を受けて、`y` 軸が
///    飽和かつ収まっている fixture に差し替えた経緯は同 test の doc参照。
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    はこの変更が入る前は `y` 軸の検出力が保たれていることの
///    check だったが、この変更でその検出力自体が失われたため、現在は同 test の
///    doc が記録するとおり別の主張 (どちらの axis も reset の理由に
///    ならない) の check になっている)。
///
///    **負方向の false positive (当初は残余リスクとして認識されていたが、
///    後に解決)**: axis 単位の gate まで閉じた上でも、飽和した axis 自身が
///    **負**の場合に固有の false positive が残っていた。当時導入されていた版の
///    再検査式 `child.location >= 0.0 && child.location <= parent.size` は、
///    `child.location` が負である間は **恒等的に false** — `>= 0.0` を満たす
///    負数は存在しないので、負方向についてこの式は「containment を
///    re-validate する」のではなく「飽和かつ負なら無条件 reset する」式と
///    数学的に同値だった。CSS Box 3 §3.1 (前掲、`margin` の負値は
///    「implementation-specific limits」の範囲で無制限に許される) の下で、
///    [`MAX_TAFFY_MAGNITUDE`] はまさにその「implementation-specific limit」
///    自身であり、そこに達したこと自体は──合法な負方向の関係が本実装の
///    上限を超えて近似され始めた、というだけで──破綻の証拠にならない。
///    これは同じ関数がすでに無条件で信頼している「飽和していない負値」
///    (`legitimate_negative_margin_overflow_is_not_reset` の `-30` や、深い
///    nest で `-30`,`-60`,…と単調に増大する中間段)と対称であり、境界
///    (`±MAX_TAFFY_MAGNITUDE` にちょうど達する瞬間) だけ扱いが不連続に
///    反転する理由が無い。
///
///    当時はここから「したがって現在の定義は符号で分岐する:
///    飽和した axis が負なら無条件に ok、正なら従来通り `<= parent.size`
///    を再検査する」という結論を導いていた。根拠は 2 つ — (a) `padding` /
///    `border` / `width` は parse 時点で非負が enforce されるため、正方向の
///    巨大な `location` を「CSS が無制限に許す」と正当化する spec 上の
///    対称な根拠が (margin とは違って) 無い、(b) 正方向の再検査は実際に
///    両方の分岐を持つ (`<=` が真になる `saturated_but_contained_layout_is_
///    not_reset`、偽になる旧
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout`) ので
///    緩めると既存の検出力を実際に失う——というものだった。
///
///    **この根拠 (a) は後に覆った**:
///    margin は symmetric — CSS Box 3 §3.1
///    (<https://www.w3.org/TR/css-box-3/#margin-physical>、"Negative values
///    for margin properties are allowed, but there may be
///    implementation-specific limits") は**負値**を明示的に許容している
///    だけで、正の margin をそれより厳しく縛る根拠にはなっていない —
///    `margin-left` の grammar (`<length-percentage> | auto`) は正負どちらの
///    巨大な値も等しく spec-legal であり、[`MAX_TAFFY_MAGNITUDE`] という
///    「implementation-specific limit」に達すること自体は、先に負方向で
///    確立したのと同じ理由で、正方向でも破綻の証拠にはならない。実際
///    `margin-left: 1e9%` (100px container 内) は `location.x` を正方向に
///    飽和させ、旧実装はこれを誤って reset していた — 実測は
///    `saturated_negative_margin_percentage_child_is_not_reset` の正方向対
///    である CSS パイプライン経由の regression test を参照。
///
///    根拠 (b) (「正方向の再検査には現に検出力がある」) はこの変更でも
///    **反証されてはいない** — 反証されたのは (a) だけで、(b) の
///    「検出力を失う」という指摘自体は正しかった。その損失は
///    **承知の上で受け入れられた** — 既存 test は TaffyLayout を直接構築する
///    のみで実 CSS パイプライン経由の検出力を一度も示しておらず、一方で
///    今回の data loss (legitimate content の完全消失) は実 CSS 経由で
///    実証済みだったため。**結果として `axis_ok` は符号を問わず「飽和して
///    いれば無条件 accept」に統一され、[`child_within_parent_border_box`]
///    は常に `true` を返す** — containment を再検査する経路は
///    もう存在しない (`axis_ok` の実装、および doc「符号を問わず無条件
///    accept になった理由」節参照)。「この invariant 自体を維持すべきか」
///    は別途明示的に deferred とされた decision であり、
///    本 doc のこの時点では未解決。
///
///    この変更で挙動が反転した regression check: 旧
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout` は
///    `saturated_child_outside_parent_is_not_reset` に改名・反転し、旧
///    `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    は
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    に改名・反転した — 詳細はそれぞれの doc を参照。
///
///    **別案として検討し却下したもの**: 「`parent.size` 自身も同じ axis で
///    飽和していれば (符号を見ずに) re-validate をスキップする」という
///    parent 飽和ゲート案は、
///    `saturated_negative_margin_percentage_child_is_not_reset`
///    (単一の `margin-left: -1e9%` 宣言、nest 無し、parent は
///    `width: 100px` で飽和していない) を誤って reset したまま説明できない
///    ——「parent が飽和しているかどうか」ではなく「child 自身の符号」が
///    正しい判別軸である証拠として、この test を上の nested chain test と
///    独立に残している。
///
/// NaN → `0.0` (`sanitize_finite` の doc参照) は飽和境界 (`±MAX_TAFFY_
/// MAGNITUDE`) に一致しないため invariant 2 の gate をすり抜けるが、実害は
/// 無い — `location=(0,0)` `size=(0,0)` は非負サイズの任意の parent に
/// 対して常に「収まっている」ので、invariant 2 が検査対象から漏れても
/// そもそも違反として検出すべき状態にならない。padding/border が巻き込
/// まれて NaN → 0 になり content box が負に振れるケースは invariant 1 が
/// 無条件に (gate なしで) 拾う。
///
/// # 決定的 fallback: subtree をゼロ化 (`zero_layout_subtree`)
///
/// 「入力制限」「途中 saturation」「layout abort」を採らず「決定的
/// fallback」を採る設計判断は決定済み。fallback 値は検討時に挙がった 2 案
/// (「0 サイズ」「直近の有限な親サイズ」) のうち **0 サイズ**を採る —
/// [`taffy::Layout::with_order`] (`order` だけ保持、他は全 zero) は
/// (a) 自明に invariant 1 (`0 - 0 - 0 = 0 >= 0`) と invariant 2 (`(0,0)` は
/// 非負サイズの任意 parent に収まる) の両方を再帰的に満たすため、subtree
/// 全体をこの値で埋めても新たな invariant 違反を作らない (「直近の有限な
/// 親サイズへの fallback」だと、fallback 後の値がさらに invariant 2 を
/// 破らないことを別途保証する必要があり、再検査を繰り返す設計になる)、
/// (b) 検討時の記述でも「0 サイズ」を先に挙げていた、の 2 点から選んだ。
///
/// # traversal は再帰しない
///
/// この関数が対象にする入力 (深い percentage nest — [`MAX_TAFFY_MAGNITUDE`]
/// の doc 表) はまさに**深い DOM tree** なので、[`find_body`] や cascade の
/// deep-nesting 対策と同じ理由で、素朴な再帰で書くと同じ入力で stack
/// overflow の新しい経路を作ってしまう。本関数と [`zero_layout_subtree`]
/// はともに `Vec` を明示 stack として使う iterative 実装。
///
/// # taffy が実際に visit した node だけを見る
///
/// `document.nodes[idx].children` をそのまま辿らず、taffy の
/// `TraversePartialTree` 実装 (`taffy_impl.rs`) と同じ `is_in_document()`
/// filter を子の走査に適用する — `<template>` descendants など taffy が
/// そもそも layout しなかった node は `unrounded_layout` が構築時デフォルト
/// (`Layout::with_order(0)`) のまま (他 subtree の古い値が紛れ込むわけでは
/// ない) なので対象に含めても実害は無いが、taffy の traversal 契約と揃えて
/// おく方が読み手にとって驚きが無い。
///
/// # 呼び出し元
///
/// [`layout_single_page`] の Step 5 (`compute_root_layout`) 直後、Step 6
/// (warning replay) の前 — root (`<body>`) の下で taffy が実際に書いた全
/// `unrounded_layout` が揃った直後に 1 回だけ走る。`compute_root_layout` を
/// 直に呼ぶ経路 (`lib.rs` の各 unit test) はこの pass を経由しない —
/// それらのテストは `sanitize_taffy_layout` (finiteness のみ) の
/// characterization が目的であり、意味的 invariant は対象外
/// (`taffy_block_layout_does_not_hang_on_raw_nonfinite_style_geometry` の
/// doc参照)。
pub(crate) fn enforce_layout_invariants(document: &mut Document, root_idx: usize) {
    let mut content_box_violations = 0usize;
    // `child_within_parent_border_box` now always
    // returns `true` (see its doc), so the `if` below that increments this
    // is unreachable for any input — `containment_violations` can never
    // exceed 0, and the `LayoutWarn::GeometryInvariantViolated { invariant:
    // "child_within_parent_border_box", .. }` warning below can never be
    // emitted. Kept (not deleted) because removing the dead branch is part
    // of the deferred "should this invariant check exist at all" follow-up.
    let mut containment_violations = 0usize;
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let layout = document.nodes[idx].unrounded_layout;
        if layout.content_box_width() < 0.0 || layout.content_box_height() < 0.0 {
            zero_layout_subtree(document, idx);
            content_box_violations += 1;
            continue;
        }
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if !document.nodes[child_idx].is_in_document() {
                continue;
            }
            let child_layout = document.nodes[child_idx].unrounded_layout;
            if !child_within_parent_border_box(&layout, &child_layout) {
                zero_layout_subtree(document, child_idx);
                containment_violations += 1;
            } else {
                stack.push(child_idx);
            }
        }
    }
    if content_box_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "content_box_non_negative",
                subtree_count: content_box_violations,
            },
        );
    }
    if containment_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "child_within_parent_border_box",
                subtree_count: containment_violations,
            },
        );
    }
}

/// [`enforce_layout_invariants`] の fallback 本体 — `root_idx` を根とする
/// subtree (taffy が実際に訪問した node のみ、`is_in_document()` filter) の
/// `unrounded_layout` を [`taffy::Layout::with_order`] (`order` だけ保持し
/// 他は全 zero) で上書きする。iterative (`Vec` stack) — 対象がまさに深い
/// DOM である以上、再帰は使わない ([`enforce_layout_invariants`] の
/// 「traversal は再帰しない」節参照)。
fn zero_layout_subtree(document: &mut Document, root_idx: usize) {
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let order = document.nodes[idx].unrounded_layout.order;
        document.nodes[idx].unrounded_layout = TaffyLayout::with_order(order);
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if document.nodes[child_idx].is_in_document() {
                stack.push(child_idx);
            }
        }
    }
}

/// [`MAX_TAFFY_MAGNITUDE`] の対称 clamp 境界にちょうど乗っているかどうか。
/// `sanitize_finite` (`v.clamp(min, max)`) は範囲外の有限値をちょうど
/// `min` / `max` に丸めるので、この等価判定は「この field で実際に clamp
/// が発火した」ことの正確な proxy になる — `NaN` は `0.0` に丸まる別経路
/// なのでここには現れない ([`enforce_layout_invariants`] の doc「NaN →
/// 0.0 …」節参照)。
fn taffy_magnitude_is_saturated(v: f32) -> bool {
    v == MAX_TAFFY_MAGNITUDE || v == -MAX_TAFFY_MAGNITUDE
}

/// `child` の border box の**原点** (`location`、`child.size` は見ない) が
/// `parent` の border box (parent 座標系の原点 `(0,0)` から `parent.size`)
/// の中にあるかどうかを検査する述語として設計された。**ただし
/// 本関数は常に `true` を返す** — 飽和して
/// いない axis は元から無条件に「ok」、飽和している axis も**符号を問わず**
/// 無条件に「ok」になったため (詳細は下の「符号を問わず無条件 accept に
/// なった理由」節)、
/// containment を実際に再検査する経路はもう存在しない。「この invariant
/// check 自体を維持すべきか」は別途明示的に deferred とされた
/// decision であり、この doc の時点では未解決 (下記参照)。
///
/// # `child.size` を見ない理由
///
/// 当初は `location.x + size.width <= parent.size.width` という **extent**
/// (child の右端/下端まで含めた) containment を検査していたが、これは
/// `width: 200%` のような「子が親より意図的に大きい」legitimate な CSS を
/// 誤検出することが実装時に判明した — `width: 200%` は**どの深さでも** (飽和
/// していない浅い段も含め) 同じ「child は parent の 2 倍」という一貫した
/// 関係を表しており、破綻ではない。深い nest で個々の used value が
/// [`MAX_TAFFY_MAGNITUDE`] の帯を超えて近似され始めても、この関係自体は
/// 変わらない (`legitimate_negative_margin_overflow_is_not_reset` が check する
/// 「小さい parent + 大きい child」も同じ class の legitimate overflow)。
/// この関数の設計もこの区別を反映しており、「child の
/// **location** が parent の border box 内」とだけ書いている — extent では
/// なく **origin** の containment を指している。この関数はその通り origin
/// だけを見る。
///
/// # gate を axis 単位・`child.location` 自身に限定する理由
///
/// 直前の版は「`parent.size.width/height` か `child.size.width/height` か
/// `child.location.x/y` のいずれか 1 つでも飽和していれば、`x`/`y` **両方**を
/// 検査する」という gate だった。この形には 2 段階の false positive があった:
///
/// 1. **field 単位**: `child.location` 自身は飽和していなくても、
///    無関係な `parent.size` (同じ subtree の別の場所の飽和が原因のことも
///    ある) や `child.size` が飽和しているだけで検査が開いてしまい、
///    legitimate な負 margin (`location.x` が通常範囲の負値、例: `-30`) を
///    `>= 0.0` で弾いて誤って reset していた。
/// 2. **axis 単位**: 1 を「`child.location` 自身が飽和していること」に
///    絞っても、`x` と `y` の**どちらか一方**が飽和していれば両方を検査する
///    形のままだと、飽和していない側の axis に legitimate な負 margin が
///    あると同じ理由で誤検出しうる。
///
/// axis 単位に絞った版はどちらも閉じていた — field 単位で
/// 「検査対象にする/しない」を区別する gate 自体は **axis 自身の
/// `child.location` が実際に飽和しているかどうか**に限っていたため、
/// legitimate な負 margin (通常範囲、飽和していない) を持つ axis が
/// 誤って巻き込まれることはなかった。飽和した field だけが「actual value
/// が taffy の生の計算結果から乖離している」ため区別対象になる、という
/// 本関数群の一貫した設計原則 (本 module doc「なぜ `sanitize_taffy_layout`
/// だけでは閉じないか」節) を axis 粒度まで徹底した形、という説明はこの
/// 時点では正確だった。**この変更以降は、飽和した axis も
/// 無条件 accept になったため、この gate は「どの axis が検査対象になるか」
/// ではなく「どの axis も検査されない」という結果に収束している** — 下の
/// 「符号を問わず無条件 accept になった理由」節参照。
///
/// `saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// は当初「飽和した axis だけ検査、他 axis は無条件 ok」という
/// conjunction を直接 check していた — 飽和している axis 自身は実際には
/// 収まっているようにし、もう一方の (飽和していない) axis に legitimate な
/// 負 margin を与えることで、「`y` 軸だけでも reset の説明がつく」fixture
/// では新旧実装を区別できないという指摘を踏まえた設計だった。この変更以降はこの test の assert 自体は
/// 変わらず通るが (fixture がたまたま「収まっている」ケースなので)、
/// 主張の中身は「どちらの axis も reset の理由にならない」に変わっている
/// (同 test の doc 参照)。旧
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// (現
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
/// は当初「`y` 軸の検出力」の check だったが、この変更でその検出力
/// 自体が失われたため reset されなくなった。旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// (現 `saturated_child_outside_parent_is_not_reset`) も同様 — 飽和した
/// axis 自身が (かつては) 違反していても、この変更以降はもう検出されない。
///
/// # 符号を問わず無条件 accept になった理由 (負方向 → 正方向の順で変更)
///
/// **負方向**: axis 単位まで絞った直後の版でも
/// なお、**飽和した axis 自身が負**のケースに固有の false positive が
/// 残っていた。再検査式 `child.location >= 0.0 && child.location <=
/// parent.size` は、`child.location` が負である限り `>= 0.0` を
/// 満たしようがないので**恒等的に false** — つまり負方向についてこの式は
/// 「containment を検査する」のではなく「飽和かつ負なら無条件に reset
/// する」ことと同値だった。CSS Box 3 §3.1 (`sanitize_taffy` の doc参照、
/// margin の負値は「implementation-specific limits」の範囲で無制限に
/// 許される) の下では、[`MAX_TAFFY_MAGNITUDE`] こそがその limit そのもので
/// あり、そこに達したこと自体は合法な負方向の関係が本実装の上限を超えて
/// 近似され始めた、というだけで破綻の証拠にはならない — 同じ関数が
/// すでに無条件で信頼している「飽和していない負値」
/// (`legitimate_negative_margin_overflow_is_not_reset` の `-30`) と対称
/// であり、`±MAX_TAFFY_MAGNITUDE` の境界を跨いだ瞬間だけ扱いを不連続に
/// 反転させる理由が無い。
///
/// 当時はここで「正方向は緩めない」と結論していた。根拠は 2 つ —
/// (a) `padding` / `border` / `width` は parse 時点で非負が enforce
/// されるため、正方向の巨大な `location` を margin と同じ「CSS が無制限に
/// 許す」根拠では正当化できない、(b) 正方向の再検査は実際に pass/fail
/// 両方の分岐を持ち (`saturated_but_contained_layout_is_not_reset` が
/// pass、旧 `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// が fail)、緩めると現に存在する検出力を失う — 負方向はそもそも pass
/// する経路が存在しなかったので、失われる検出力は無い、というものだった。
///
/// **正方向**: 根拠 (a) は
/// 覆った — margin は symmetric。CSS Box 3 §3.1
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>、"Negative values
/// for margin properties are allowed, but there may be
/// implementation-specific limits") は**負値**を明示的に許容している
/// だけで、正の margin をそれより厳しく縛る spec 上の対称な根拠にはなって
/// いない。`margin-left: 1e9%` (`width: 100px` container 内) は
/// `location.x` を正方向に飽和させ、旧実装はこれを誤って reset していた —
/// `saturated_positive_margin_percentage_child_is_not_reset` が実際の
/// CSS パイプライン経由でこれを check する (`saturated_negative_margin_
/// percentage_child_is_not_reset` の正方向対)。根拠 (b) (「検出力を失う」)
/// は反証されていない — その損失は承知の上で受け入れられた: 既存 test
/// (`saturated_but_contained_layout_is_not_reset`、旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`) は
/// TaffyLayout を直接構築するのみで実 CSS パイプライン経由の検出力を
/// 一度も示していなかった一方、今回の data loss (legitimate content の
/// 完全消失) は実 CSS 経由で実証済みだったため。
///
/// **結果**: `axis_ok` は符号を問わず「飽和していれば無条件 accept」に
/// 統一され、本関数は常に `true` を返す — containment を再検査
/// する経路はもう存在しない。旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout` は
/// `saturated_child_outside_parent_is_not_reset` に、旧
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// は
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// に、それぞれ改名・反転した (詳細は各 test の doc参照)。「この
/// invariant check 自体を維持すべきか」は別途明示的に deferred
/// とされた decision であり、この doc の時点では未解決。
///
/// `saturated_negative_margin_percentage_child_is_not_reset` (単一の
/// `margin-left: -1e9%` 宣言、nest 無し) と
/// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset`
/// (`width: 200%; margin-left: -100%` の深い nest chain、
/// `nested_percentage_wide_child_chain_is_not_reset` の負方向対) が
/// 実際の CSS パイプライン経由でこれを check する。前者は特に、「`parent.size`
/// 自身も同じ axis で飽和していれば符号を見ずに re-validate をスキップ
/// する」という検討したが却下した別案を反証する最小 fixture でもある —
/// この fixture は `parent.size.width` が飽和していない (`100.0` のまま)
/// ので、判別軸は「parent も飽和しているか」ではなく「child 自身の符号」
/// でなければならないことを示す (この変更以降、この判別軸自体は意味を失った
/// が、fixture と regression check としての価値は変わらない)。
/// `saturated_negative_location_is_not_reset` は同じ組み合わせを直接構築
/// した最小 synthetic case で孤立させて検査する
/// (`saturated_but_contained_layout_is_not_reset` と対になる、正方向
/// ケースの負方向対 — この変更以降はどちらも「飽和していれば無条件 accept」
/// という同じ結論の pin)。
fn child_within_parent_border_box(parent: &TaffyLayout, child: &TaffyLayout) -> bool {
    /// 1 axis 分の判定。`location` はその axis の `child.location.{x,y}`
    /// (`parent_size` は本体の式ではもう一切使わない — dead parameter。
    /// 削除せず「対応する `parent.size.{width,height}` を渡す」という
    /// 呼び出し規約の見た目だけ残しているのは、この関数・`axis_ok` 自体を
    /// 削除するかどうかを含めて「この invariant check を維持すべきか」が
    /// 別途明示的に deferred とされた follow-up だから — 将来その follow-up
    /// で `axis_ok` ごと削除される可能性があることを見越して、今
    /// signature を先回りして変える判断はしていない。詳細は上の doc
    /// 「符号を問わず無条件 accept になった理由」節。同じ理由で、直下の
    /// `if !taffy_magnitude_is_saturated(location) { return true; }` 分岐
    /// も実質的には常に `true` を返す既定文と等価な dead branch になって
    /// いる — こちらも `axis_ok` ごと削除されうる同じ deferred follow-up
    /// まで、あえて `true` 一本に畳んでいない)。
    fn axis_ok(location: f32, _parent_size: f32) -> bool {
        if !taffy_magnitude_is_saturated(location) {
            return true;
        }
        // 飽和していれば符号を問わず無条件 accept。
        // 負方向は先に確立していた — 本 decision は
        // その前例を正方向にも対称に拡張し、旧 `location <= parent_size`
        // 再検査 (正方向限定) を撤去した。containment を再検査する経路は
        // もう存在しない。
        true
    }
    axis_ok(child.location.x, parent.size.width) && axis_ok(child.location.y, parent.size.height)
}

#[cfg(test)]
mod tests;
