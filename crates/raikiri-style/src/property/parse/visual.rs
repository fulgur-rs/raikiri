//! Visual property parsers: backgrounds, gradients, `<position>`, clip-path
//! shapes, transforms, filters, masks, opacity and compositing.

use std::sync::Arc;

use cssparser::{ParseError, Parser, Token};

use crate::property::types::*;

use super::color::*;
use super::common::*;
use super::text::*;

/// `<opacity-value> = <number> | <percentage>` (CSS Color 4 §3.3
/// "Transparency: the opacity property"
/// <https://www.w3.org/TR/css-color-4/#transparency>). `<percentage>` uses
/// `expect_percentage`'s `unit_value` (already divided by 100) verbatim, no
/// 0%..100% range check.
///
/// # Deliberately does not clamp
///
/// Same §, verbatim: "Opacity values outside the range \[0, 1\] are not
/// invalid, and are preserved in specified values, but are clamped to the
/// range \[0, 1\] in computed values." Clamping is a **computed-value-time**
/// transform, so this parser — unlike [`parse_alpha_value`] (the `<alpha-value>`
/// grammar `rgb()`/`rgba()` use for their alpha channel, which shares the
/// exact same `<number> | <percentage>` grammar but clamps immediately at
/// parse time) — must preserve an out-of-range parse as-is. The two helpers
/// are kept separate rather than shared despite the identical grammar,
/// because sharing would silently store an already-clamped value at the
/// specified layer. The actual clamp lives in
/// [`crate::specified::SpecifiedValues::absolutize_with`] (phase 3) and its
/// page-context sibling.
///
/// # `!is_nan()` guard — NaN, but *not* `+Inf`/`-Inf`, must be rejected
///
/// `+Inf`/`-Inf` are spec-valid `<number>` values (CSS Color 4 §3.3 puts no
/// range restriction on `<opacity-value>`'s `<number>` alternative) and, per
/// CSS Values and Units Module Level 4 §5 "Numeric Data Types"
/// (<https://www.w3.org/TR/css-values-4/#numeric-types>), when a literal
/// exceeds the implementation's supported precision it "must be converted to
/// the closest value supported by the implementation" where "how the
/// implementation defines 'closest' is implementation-defined" — treating
/// IEEE-754 `Infinity` as that closest value (rather than e.g. `f32::MAX`)
/// is a deliberate, spec-permitted choice, not the only conformant answer,
/// and is what cssparser's `f64`->`f32` overflow naturally produces. They are
/// handled correctly by the phase-3 clamp above — `f32::clamp` maps
/// `+Inf`/`-Inf` to `1.0`/`0.0` exactly as it maps any other out-of-range
/// finite value, so a huge-magnitude literal like `opacity: -1e40` (which
/// the tokenizer's f64->f32 conversion overflows to `-Infinity`, not NaN)
/// must reach the clamp unrejected and become `0.0` (fully transparent),
/// not fall back to the initial `1.0` (fully opaque) by being dropped
/// here. **NaN is different** — `f32::clamp` returns `self` unchanged when
/// `self` is NaN (only a NaN *bound* panics), so an unguarded NaN would
/// sail through the clamp and reach
/// [`crate::computed::ComputedValues::opacity`] — this guard is what keeps
/// that field NaN-free for values that go through this parser (see that
/// field's doc for the "ordinary parse -> cascade pipeline" scoping of
/// that guarantee). An earlier iteration of this guard used `is_finite()`,
/// which rejects `+Inf`/`-Inf` too and wrongly dropped `-1e40`-shaped
/// declarations — this helper deliberately does **not** reuse
/// [`parse_nonneg_finite_number`](super::layout::parse_nonneg_finite_number)'s `is_finite()` pattern for that reason;
/// unlike `flex-grow`/`flex-shrink`, which reject `<number [0,∞]>` and so
/// have no legitimate use for `-Inf` in the first place, `opacity`'s
/// unbounded `<number>` grammar makes `+Inf`/`-Inf` legitimate inputs that
/// must reach the clamp.
///
/// `expect_number_stable`/`expect_percentage_stable` already correct the
/// one class of `NaN` a numeric token can actually carry (a huge-exponent,
/// zero-mantissa literal like `opacity: 0e999`, module doc above) before
/// this function ever sees the value, so in ordinary use `n`/`pct` here
/// are never `NaN`. This `!is_nan()` check is kept as defense-in-depth —
/// it costs nothing when the value isn't `NaN` and still protects
/// [`crate::computed::ComputedValues::opacity`]'s NaN-free invariant if
/// that upstream recovery is ever bypassed or extended incorrectly.
pub(super) fn parse_opacity_value(input: &mut Parser<'_, '_>) -> Option<f32> {
    let val = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        if pct.is_nan() {
            return None;
        }
        pct
    } else {
        let n = expect_number_stable(input).ok()?;
        if n.is_nan() {
            return None;
        }
        n
    };
    if input.try_parse(|i| i.expect_exhausted()).is_err() {
        return None;
    }
    Some(val)
}

/// `visibility: <ident>` を parse する (CSS Display 3 §4
/// <https://www.w3.org/TR/css-display-3/#visibility>)。
///
/// Value grammar (spec verbatim): `visible | hidden | collapse`。全 3
/// keyword を受理する ([`Visibility`] doc の Scope carving 節参照 —
/// `collapse` の formatting-context 固有な space-saving 効果は未実装だが、
/// keyword 自体は spec-valid として受理する)。ASCII case-insensitive で
/// ident を比較する ([`parse_font_style`] と同 flavor)。
pub(super) fn parse_visibility(input: &mut Parser<'_, '_>) -> Option<Visibility> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        "hidden" => Some(Visibility::Hidden),
        "collapse" => Some(Visibility::Collapse),
        _ => None,
    }
}

/// CSS `<url>` value type (CSS Values and Units 4 §4.4
/// <https://www.w3.org/TR/css-values-4/#urls>) を一般 property value として
/// 受理する共通 helper。
///
/// Grammar: `<url> = <url()> | <src()>`、
/// `<url()> = url( <string> <url-modifier>* ) | <url-token>`。本 helper は
/// `<url()>` の 2 形式のみを受理する — unquoted `url(foo.png)` (tokenizer が
/// 生成する `<url-token>`) と、quoted `url("foo.png")` (tokenizer は quote を
/// 見て `url` を function token として切り出し、nested block 内の
/// `<string-token>` を読む)。`<url-modifier>*` (`crossorigin()` 等、§4.4.1) を
/// 伴う `url()` は nested block が `<string>` 単体で exhaust しないため
/// 全体が reject される — cssparser の block-exhaustion 制約
/// (`Parser::parse_nested_block` は closure が block を完全消費しないと Err)
/// によるもので、意図的な未対応。`<src()>` は別理由で未対応 —
/// `expect_url()` は function token 名を `url` に限定するため、`src(...)` は
/// そもそも function token としてすら認識されず reject される
/// (block-exhaustion 制約とは無関係)。
///
/// 一般 property value としての `<url>` はこの 2 形式のみで、bare
/// `<string>` (quote だけで `url()` wrapper を伴わない形) は含まない — spec
/// 本文が `"Some CSS contexts (such as @import) also allow a <url> to be
/// represented by a bare <string>, without the function wrapper"` と明記する
/// 通り、bare string 受理は `@import` 等の特定 context に限定された legacy
/// 挙動であり、一般 property の `<url>` value type には及ばない。
/// (`[ <string> | <url> ]` を独立した alternative として明示的に持つ property
/// は [`parse_target_url`](super::content::parse_target_url) のように呼び出し側で `<string>` を別途扱う。)
///
/// CSS-wide keyword (`inherit` 等の bare ident) はこの grammar のどの形にも
/// 一致しないため cssparser の `expect_url` が Err を返し、本 helper も
/// `None` を返す — 呼び出し元の `parse_value` 規約 (`None` で declaration
/// 全体を drop) と自然に合致する。
// `pub(crate)` ではなく plain `fn`: 呼び出し元 (`parse_background_image`) は
// property.rs 内に実装されており、外部 module からの呼び出し元は無い —
// 外部呼び出し元が実際に landing した時点で
// `parse_non_negative_length`/`parse_length_allow_negative` (このファイル内、
// 外部呼び出し元を doc に明記した precedent) と同様の `pub(crate)` + doc
// justification へ拡張する。
pub(crate) fn parse_url_value(input: &mut Parser<'_, '_>) -> Option<String> {
    input.expect_url().ok().map(|s| s.as_ref().to_string())
}

/// `<length-percentage>` — sign 制限なし ([`parse_length_value`] with
/// `allow_percentage=true`、`Result` wrapper for `try_parse` callers)。
/// [`CssPosition`] の offset (負値も spec-valid、[`parse_position_horizontal_edge`]
/// 等の doc 参照) が使う。
pub(super) fn parse_length_percentage_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_length_value(input, true).ok_or_else(|| input.new_custom_error(()))
}

/// [`CssPositionOffset::End`] の `Percent` payload を、等価な
/// [`CssPositionOffset::Start`] へ畳む ([`CssPositionOffset`] doc の
/// 「なぜ 2 variant か」節)。`right 30%` (`End(Percent(30.0))`) は
/// `Start(Percent(70.0))` と完全に等価な値なので、常に `Start` 側へ
/// 正規化して代表形を 1 つに保つ — `End` が生き残るのは percentage で
/// 表現できない offset (`right 10px` 等) に限られる。
fn normalize_css_position_offset(offset: CssPositionOffset) -> CssPositionOffset {
    match offset {
        CssPositionOffset::End(Length::Percent(p)) => {
            CssPositionOffset::Start(Length::Percent(100.0 - p))
        }
        other => other,
    }
}

pub(crate) fn normalize_css_position(position: CssPosition) -> CssPosition {
    CssPosition {
        horizontal: normalize_css_position_offset(position.horizontal),
        vertical: normalize_css_position_offset(position.vertical),
    }
}

fn css_position_center() -> CssPositionOffset {
    CssPositionOffset::Start(Length::Percent(50.0))
}

/// `<position>` grammar ([`CssPosition`] doc) の edge + optional offset
/// を読む共通ヘルパー。`start_kw` (`left`/`top`) は [`CssPositionOffset::Start`]、
/// `end_kw` (`right`/`bottom`) は [`CssPositionOffset::End`] にマップする。
/// 戻り値の 2nd 要素の意味は [`parse_position_horizontal_edge`] doc 参照。
fn parse_position_edge(
    input: &mut Parser<'_, '_>,
    start_kw: &str,
    end_kw: &str,
) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching(start_kw))
        .is_ok()
    {
        return Some(match input.try_parse(parse_length_percentage_res) {
            Ok(offset) => (CssPositionOffset::Start(offset), true),
            Err(_) => (CssPositionOffset::Start(Length::Percent(0.0)), false),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching(end_kw)).is_ok() {
        return Some(match input.try_parse(parse_length_percentage_res) {
            Ok(offset) => (CssPositionOffset::End(offset), true),
            Err(_) => (CssPositionOffset::End(Length::Percent(0.0)), false),
        });
    }
    None
}

/// `<position>` grammar ([`CssPosition`] doc) の `[ left | right ]
/// <length-percentage>?` alternative — horizontal 軸の edge keyword を
/// optional offset とともに読む。offset 省略時は edge そのもの (offset
/// `0`) を返す。`left`/`right` どちらにもマッチしなければ `None`
/// (token は消費しない)。
///
/// 戻り値の 2nd 要素は「`<length-percentage>` token が実際に authored
/// されていたか (offset 省略時の暗黙 `0` ではない)」— [`parse_position_branch3`]
/// (`<bg-position>` 用、`background-position` が使う) はこれを無視するが、
/// [`parse_position_branch3_strict`] (plain `<position>` 用、`object-position`
/// が使う) はこれを使って 3-value 形式 (offset がどちらか片方の軸にだけ
/// authored されている状態、`<position>` doc の Grammar 節参照) を検出・
/// reject する。
fn parse_position_horizontal_edge(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    parse_position_edge(input, "left", "right")
}

/// [`parse_position_horizontal_edge`] の vertical 軸版 (`top`/`bottom`) —
/// 戻り値の 2nd 要素の意味は同関数の doc 参照。
fn parse_position_vertical_edge(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    parse_position_edge(input, "top", "bottom")
}

/// `center | [ left | right ] <length-percentage>?` — horizontal 軸の
/// branch-3 group ([`parse_position_branch3`] doc)。`center` は
/// offset を持たないため 2nd 要素は常に `false`
/// ([`parse_position_horizontal_edge`] doc参照)。
fn parse_position_horizontal_group(
    input: &mut Parser<'_, '_>,
) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some((css_position_center(), false));
    }
    parse_position_horizontal_edge(input)
}

/// [`parse_position_horizontal_group`] の vertical 軸版。
fn parse_position_vertical_group(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some((css_position_center(), false));
    }
    parse_position_vertical_edge(input)
}

/// [`parse_position_branch3`] と [`parse_position_branch3_strict`] の共通コア —
/// `<position>` 3rd alternative の構造を一度だけ記述し、offset 有無
/// (`bool` 2 つ) を呼び出し元へ返す。`center` は常に offset なし (`false`)
/// ([`parse_position_horizontal_group`] doc 参照)。
fn parse_position_branch3_core(input: &mut Parser<'_, '_>) -> Option<(CssPosition, bool, bool)> {
    if let Some((horizontal, h_offset)) = parse_position_horizontal_edge(input) {
        let (vertical, v_offset) = parse_position_vertical_group(input)?;
        return Some((
            CssPosition {
                horizontal,
                vertical,
            },
            h_offset,
            v_offset,
        ));
    }
    if let Some((vertical, v_offset)) = parse_position_vertical_edge(input) {
        let (horizontal, h_offset) = parse_position_horizontal_group(input)?;
        return Some((
            CssPosition {
                horizontal,
                vertical,
            },
            h_offset,
            v_offset,
        ));
    }
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        if let Some((vertical, v_offset)) = parse_position_vertical_edge(input) {
            return Some((
                CssPosition {
                    horizontal: css_position_center(),
                    vertical,
                },
                false,
                v_offset,
            ));
        }
        if let Some((horizontal, h_offset)) = parse_position_horizontal_edge(input) {
            return Some((
                CssPosition {
                    horizontal,
                    vertical: css_position_center(),
                },
                h_offset,
                false,
            ));
        }
        if input
            .try_parse(|i| i.expect_ident_matching("center"))
            .is_ok()
        {
            return Some((
                CssPosition {
                    horizontal: css_position_center(),
                    vertical: css_position_center(),
                },
                false,
                false,
            ));
        }
        return None;
    }
    None
}

/// `<position>` grammar ([`CssPosition`] doc) の 3rd alternative —
/// `[ center | [ left | right ] <length-percentage>? ] && [ center | [ top |
/// bottom ] <length-percentage>? ]`。`&&` は両 group が (任意順で) 必須
/// なことを意味する — 一方の group しか読めなければ (呼び出し元が
/// `input.try_parse` で包む前提の) `None` を返し、消費した token は
/// 呼び出し元の rewind に委ねる。
///
/// 1 個目の token は horizontal-exclusive (`left`/`right`) → vertical-exclusive
/// (`top`/`bottom`) → ambiguous `center` の順で試す。`center` は両 group に
/// 属し得るため、どちらの軸に属するかは 2 個目の token (もう片方の
/// group) を見て初めて決まる。
///
/// これは `<bg-position>` (CSS Backgrounds 3 §2.6) の 3rd alternative
/// そのもの — 各 group の `<length-percentage>?` を独立に optional として
/// 扱う (offset がどちらか片方の軸にだけ authored される 3-value 形式を
/// 受理する)。plain `<position>` (CSS Values 4 §8.3) 向けにはこの中間形を
/// reject する [`parse_position_branch3_strict`] を使うこと —
/// `background-position` (`<bg-position>` を要求) はこちら、
/// `object-position` (`<position>` を要求) はあちら、という使い分けが
/// [`CssPosition`] の Grammar 節の canonical な説明。
fn parse_position_branch3(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let (position, _, _) = parse_position_branch3_core(input)?;
    Some(position)
}

/// [`parse_position_branch3`] の plain-`<position>` 版 —
/// 唯一の違いは、horizontal/vertical 両 group の `<length-percentage>`
/// offset **有無が一致しない場合 (3-value 形式) を reject** する点。
///
/// # Why
///
/// CSS Values 4 §8.3 の `<position>` 自体には「offset がどちらか片方の
/// 軸にだけ authored される」中間形が存在しない — その 4th alternative
/// (`<position-four>`、MDN "`<position>` CSS type" の formal syntax 参照)
/// は `[[left|right] <length-percentage>] && [[top|bottom]
/// <length-percentage>]` であり、offset は `?` ではなく両軸とも必須。
/// offset を一切伴わない bare keyword pair は 2nd alternative
/// (`<position-two>` の `&&` 形) が別途カバーする。つまり有効な組み合わせは
/// 「両軸とも offset あり (4-value)」か「両軸とも offset なし」のみで、
/// 「片方だけ offset」は常に invalid。
///
/// `<bg-position>` (CSS Backgrounds 3 §2.6、`background-position` 用) は
/// この制約を持たない — 3-value 形式 ("For 3-value productions (which are
/// not valid in `<position>`)" と同 spec が明記) を明示的に許すのが
/// `<bg-position>` の `<position>` に対する拡張そのもの。`object-position`
/// (CSS Images 3 §5.2、Value: `<position>`) はこの拡張を持たないため、
/// [`parse_position_branch3`] をそのまま再利用すると `right 10px center`
/// のような 3-value 入力を誤って受理してしまう — token を過不足なく
/// 消費してしまう (leftover が残らない) ため、呼び出し元の
/// `expect_exhausted` による leftover 検出でも捕捉できない。本関数は
/// それを防ぐための、offset 有無の対称性チェックを追加した sibling。
pub(crate) fn parse_position_branch3_strict(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let (position, h_offset, v_offset) = parse_position_branch3_core(input)?;
    if h_offset != v_offset {
        return None;
    }
    Some(position)
}

/// `[ <start_kw> | center | <end_kw> | <length-percentage> ]` — `<position>`
/// grammar ([`CssPosition`] doc) の 2nd alternative の axis 共通ヘルパー。
/// `start_kw` は `Start(0%)` (`left`/`top`)、`end_kw` は `Start(100%)`
/// (`right`/`bottom`) にマップする。bare `<length-percentage>` はこの
/// alternative でのみ受理される (branch3 の group はどちらも bare LP を
/// 単独では受理しない) — edge keyword が無いぶん offset ではなく「値そのもの」
/// として `Start` に格納する。
fn parse_position_branch2_axis(
    input: &mut Parser<'_, '_>,
    start_kw: &str,
    end_kw: &str,
) -> Option<CssPositionOffset> {
    if input
        .try_parse(|i| i.expect_ident_matching(start_kw))
        .is_ok()
    {
        return Some(CssPositionOffset::Start(Length::Percent(0.0)));
    }
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some(css_position_center());
    }
    if input.try_parse(|i| i.expect_ident_matching(end_kw)).is_ok() {
        return Some(CssPositionOffset::Start(Length::Percent(100.0)));
    }
    let lp = input.try_parse(parse_length_percentage_res).ok()?;
    Some(CssPositionOffset::Start(lp))
}

/// `[ left | center | right | <length-percentage> ]` — `<position>` grammar
/// ([`CssPosition`] doc) の 2nd alternative の horizontal 側。bare
/// `<length-percentage>` はこの alternative でのみ受理される (branch3 の
/// group はどちらも bare LP を単独では受理しない) — edge keyword が無い
/// ぶん offset ではなく「値そのもの」として `Start` に格納する。
fn parse_position_branch2_horizontal(input: &mut Parser<'_, '_>) -> Option<CssPositionOffset> {
    parse_position_branch2_axis(input, "left", "right")
}

/// [`parse_position_branch2_horizontal`] の vertical 側
/// (`[ top | center | bottom | <length-percentage> ]`).
fn parse_position_branch2_vertical(input: &mut Parser<'_, '_>) -> Option<CssPositionOffset> {
    parse_position_branch2_axis(input, "top", "bottom")
}

/// `<position>` grammar ([`CssPosition`] doc) の 2nd alternative — 厳密に
/// horizontal → vertical の順で 2 token を読む (keyword 並び替え不可、
/// branch3 と違い `&&` ではなく単純な連接)。
fn parse_position_branch2(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let horizontal = parse_position_branch2_horizontal(input)?;
    let vertical = parse_position_branch2_vertical(input)?;
    Some(CssPosition {
        horizontal,
        vertical,
    })
}

/// `<position>` grammar ([`CssPosition`] doc) の 1st alternative — 単一
/// keyword または単一 `<length-percentage>`。指定されなかった軸は
/// `center` (50%) になる (spec 明示なし、`background-position` (CSS
/// Backgrounds 3 §2.6) の "if only one value is specified, the second
/// value is assumed to be center" 相当を汎用 `<position>` 型として保持)。
fn parse_position_branch1(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: css_position_center(),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
        return Some(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: css_position_center(),
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("right"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: css_position_center(),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching("top")).is_ok() {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("bottom"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        });
    }
    let lp = input.try_parse(parse_length_percentage_res).ok()?;
    Some(CssPosition {
        horizontal: CssPositionOffset::Start(lp),
        vertical: css_position_center(),
    })
}

/// `<position>` value type を parse する ([`CssPosition`] doc の grammar
/// 参照)。
///
/// # Alternative の試行順序 — 3rd → 2nd → 1st (grammar 記載順とは逆)
///
/// 3 alternative は互いに重なりうるため、試す順序が結果を左右する。
/// **3rd (edge 並び替え可) を最初に試す**理由 — 3rd の失敗が「安全に
/// rewind する」ことを保証するメカニズムの説明:
///
/// `left 10px` を例にとる。3rd alternative は horizontal group に `left`
/// を、続けて optional offset `10px` を貪欲に割り当てる — その結果
/// vertical group に残す token が無くなり (`&&` は両 group 必須)、3rd
/// alternative 全体が失敗して丸ごと rewind する (この入力に限れば 2nd を
/// 先に試しても `[left|center|right|<LP>]` が `left` を、
/// `[top|center|bottom|<LP>]` が残りの `10px` を bare
/// `<length-percentage>` として消費し、同じ horizontal = `left` (0%)、
/// vertical = `10px` に到達するため、この特定の入力だけでは順序は
/// 結果を左右しない — 3rd が「余計に消費してから失敗する」ことはあっても
/// 「誤った値で成功する」ことは無い、という下記の safety-invariant の
/// 具体例として引いている)。順序が真に結果を左右するのは `top left` の
/// ような keyword 並び替え入力 (2nd は horizontal→vertical の固定順しか
/// 受理しないため `top` を horizontal 側で reject し、3rd の `&&`
/// (任意順) でしか解釈できない) や、`bottom 10px right 20px` のような
/// 3-4 value edge-offset 入力 (2nd は高々 2 token しか消費しないため
/// leftover が残り `expect_exhausted` で丸ごと drop される) である。
///
/// 同じ理由で 2nd は 1st より先: 1st は token を 1 個しか消費しないため、
/// 2 token 以上の入力 (`0px 20px` 等) に対して 2nd を先に試さないと
/// 2 個目の value が leftover として残り、呼び出し元の `expect_exhausted`
/// (`rule.rs`) が declaration ごと drop してしまう。
///
/// この順序が「3rd が prefix だけ食って leftover を残す」場合でも安全な
/// 理由: 2nd は高々 2 token、1st は高々 1 token しか消費できないため、
/// 同じ開始位置から 2nd/1st が 3rd より **多くの** token を消費して
/// leftover をゼロにできることは無い — 3rd が成功した時点でそれが
/// 常に「最も leftover が少ない (またはゼロの)」alternative になる。
///
/// 各 alternative は `input.try_parse` で包まれた `Option`-returning
/// helper (`parse_position_branch3` 等) として実装し、途中まで token を
/// 消費して失敗しても呼び出し元の `try_parse` が丸ごと rewind する。
fn parse_position_branch3_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch3(input).ok_or_else(|| input.new_custom_error(()))
}

fn parse_position_branch2_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch2(input).ok_or_else(|| input.new_custom_error(()))
}

fn parse_position_branch1_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch1(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<bg-position>` (CSS Backgrounds 3 §2.6) value type を parse する — plain `<position>` (CSS Values 4 §8.3) の superset で、3-value edge-offset 形式を許す。`background-position` / `background` shorthand の position 部分 / gradient の `at <position>` 等がこの grammar を使う。素の `<position>` (3-value 形式を reject) は sibling の [`parse_position_strict`] を使うこと。
///
/// 3 alternative ([`parse_position_branch3`] / [`parse_position_branch2`] / [`parse_position_branch1`])
/// を 3rd → 2nd → 1st の順で試す (詳細は [`parse_position_branch3_res`] の alternative 順序 doc 参照)。
pub fn parse_bg_position(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let position = input
        .try_parse(parse_position_branch3_res)
        .or_else(|_| input.try_parse(parse_position_branch2_res))
        .or_else(|_| input.try_parse(parse_position_branch1_res))
        .ok()?;
    Some(normalize_css_position(position))
}

fn parse_position_branch3_strict_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch3_strict(input).ok_or_else(|| input.new_custom_error(()))
}

/// plain `<position>` (CSS Values 4 §8.3) value type を parse する —
/// [`parse_bg_position`] (`<bg-position>`、`background-position` 用) の
/// sibling。`object-position` (CSS Images 3 §5.2、Value: `<position>`) が
/// 使う。
///
/// 差分は 3rd alternative だけ — [`parse_position_branch3`] の代わりに
/// [`parse_position_branch3_strict`] を試す (3-value edge-offset 形式を
/// reject する、同関数 doc の "Why" 節参照)。2nd/1st alternative
/// ([`parse_position_branch2`]/[`parse_position_branch1`]) はどちらの
/// grammar でも同一なので共有する。
pub fn parse_position_strict(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let position = input
        .try_parse(parse_position_branch3_strict_res)
        .or_else(|_| input.try_parse(parse_position_branch2_res))
        .or_else(|_| input.try_parse(parse_position_branch1_res))
        .ok()?;
    Some(normalize_css_position(position))
}

/// `background-image: <bg-image>` を parse する ([`BackgroundImage`] doc の
/// grammar 参照: `<image> | none`、`<image> = <url> | <gradient>`)。
///
/// `none` keyword を先に試し、次に `<gradient>` function を試す
/// (`<url>` 側 [`parse_url_value`] はどの function token にも一致しないため
/// 順序自体は結果を左右しないが、他の keyword-vs-function alternative を持つ
/// sibling parser — [`parse_position`](super::box_model::parse_position) 等 — と並びを揃える)。
pub(super) fn parse_background_image(input: &mut Parser<'_, '_>) -> Option<BackgroundImage> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(BackgroundImage::None);
    }
    if let Ok(gradient) = input.try_parse(parse_gradient) {
        return Some(BackgroundImage::Gradient(gradient));
    }
    parse_url_value(input).map(BackgroundImage::Url)
}

/// `mask-image: <mask-reference>` (single layer — [`MaskImage`] doc の
/// scope carving 節参照) を parse する。
///
/// grammar shape (`none | <image> | <mask-source>`、`<image> = <url> |
/// <gradient>`、`<mask-source> = <url>`) は [`parse_background_image`]'s
/// `<bg-image> = <url> | <gradient>` と concrete syntax レベルで一致する
/// ([`MaskImage`] doc の「`<mask-source>` と `<image>` の `url`
/// alternative は同じ具象構文」節) — [`BackgroundImage`] への alias により
/// body も共有する。
pub(crate) fn parse_mask_image(input: &mut Parser<'_, '_>) -> Option<MaskImage> {
    parse_background_image(input)
}

/// `<geometry-box>` を parse する ([`GeometryBox`] doc の grammar 参照: 7
/// keyword)。
fn parse_geometry_box(input: &mut Parser<'_, '_>) -> Option<GeometryBox> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "border-box" => Some(GeometryBox::BorderBox),
        "padding-box" => Some(GeometryBox::PaddingBox),
        "content-box" => Some(GeometryBox::ContentBox),
        "margin-box" => Some(GeometryBox::MarginBox),
        "fill-box" => Some(GeometryBox::FillBox),
        "stroke-box" => Some(GeometryBox::StrokeBox),
        "view-box" => Some(GeometryBox::ViewBox),
        _ => None,
    }
}

/// `clip-path: <clip-source> | [ <basic-shape> || <geometry-box> ] | none`
/// ([`ClipPath`] doc参照) を parse する。
///
/// `none` → `<clip-source>` (`<url>`) → `[ <basic-shape> || <geometry-box> ]`
/// の順で試す。最後の `[ … || … ]` は either order (shape before box or box
/// before shape) を許すため、両順を試す。`basic-shape` function token と
/// geometry-box ident は互いに排他的なので試行順は結果を左右しない。
pub(super) fn parse_clip_path(input: &mut Parser<'_, '_>) -> Option<ClipPath> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(ClipPath::None);
    }
    // Try the basic-shape / geometry-box pair — either order, at least one.
    // First try basic-shape with optional trailing geometry-box.
    if let Ok(shape_with_box) = input.try_parse(|i| -> Result<ClipPath, ParseError<'_, ()>> {
        let shape = parse_basic_shape(i).ok_or_else(|| i.new_custom_error(()))?;
        let geometry_box = i
            .try_parse(|j| -> Result<GeometryBox, ParseError<'_, ()>> {
                parse_geometry_box(j).ok_or_else(|| j.new_custom_error(()))
            })
            .ok();
        Ok(ClipPath::BasicShape {
            shape: Box::new(shape),
            geometry_box,
        })
    }) {
        return Some(shape_with_box);
    }
    // Then try geometry-box with optional trailing basic-shape (the other order).
    if let Ok(box_with_shape) = input.try_parse(|i| -> Result<ClipPath, ParseError<'_, ()>> {
        let geometry_box = parse_geometry_box(i).ok_or_else(|| i.new_custom_error(()))?;
        if let Ok(shape) = i.try_parse(|j| -> Result<BasicShape, ParseError<'_, ()>> {
            parse_basic_shape(j).ok_or_else(|| j.new_custom_error(()))
        }) {
            Ok(ClipPath::BasicShape {
                shape: Box::new(shape),
                geometry_box: Some(geometry_box),
            })
        } else {
            Ok(ClipPath::GeometryBox(geometry_box))
        }
    }) {
        return Some(box_with_shape);
    }
    parse_url_value(input).map(ClipPath::Url)
}

/// `<basic-shape>` — `circle()` / `ellipse()` / `inset()` / `polygon()` / `path()`.
fn parse_basic_shape(input: &mut Parser<'_, '_>) -> Option<BasicShape> {
    let name = input
        .try_parse(|i| -> Result<String, ParseError<'_, ()>> {
            let token = i.next()?.clone();
            match token {
                Token::Function(n) => Ok(n.as_ref().to_ascii_lowercase()),
                _ => Err(i.new_unexpected_token_error(token)),
            }
        })
        .ok()?;
    match name.as_str() {
        "circle" => input
            .parse_nested_block(|i| -> Result<CircleShape, ParseError<'_, ()>> {
                parse_circle_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Circle),
        "ellipse" => input
            .parse_nested_block(|i| -> Result<EllipseShape, ParseError<'_, ()>> {
                parse_ellipse_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Ellipse),
        "inset" => input
            .parse_nested_block(|i| -> Result<InsetShape, ParseError<'_, ()>> {
                parse_inset_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Inset),
        "polygon" => input
            .parse_nested_block(|i| -> Result<PolygonShape, ParseError<'_, ()>> {
                parse_polygon_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Polygon),
        "path" => input
            .parse_nested_block(|i| -> Result<PathShape, ParseError<'_, ()>> {
                parse_path_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Path),
        _ => None,
    }
}

fn parse_fill_rule(input: &mut Parser<'_, '_>) -> Option<FillRule> {
    let ident = input.expect_ident().ok()?.as_ref().to_ascii_lowercase();
    match ident.as_str() {
        "nonzero" => Some(FillRule::NonZero),
        "evenodd" => Some(FillRule::EvenOdd),
        _ => None,
    }
}

/// Try to parse `<shape-radius>` as `closest-side` / `farthest-side` or `<length-percentage [0,∞]>`.
fn try_parse_shape_radius(input: &mut Parser<'_, '_>) -> Option<ShapeRadius> {
    if let Ok(sr) = input.try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.as_ref().to_ascii_lowercase();
        match ident.as_str() {
            "closest-side" => Ok(ShapeRadius::ClosestSide),
            "farthest-side" => Ok(ShapeRadius::FarthestSide),
            _ => Err(i.new_custom_error(())),
        }
    }) {
        return Some(sr);
    }
    let length = parse_length_value(input, true)?;
    if length.payload() < 0.0 || length.payload().is_nan() {
        return None;
    }
    Some(ShapeRadius::Length(length))
}

fn parse_circle_shape(input: &mut Parser<'_, '_>) -> Option<CircleShape> {
    let radius = input
        .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
            try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    let position = if input.try_parse(|i| i.expect_ident_matching("at")).is_ok() {
        let pos = parse_position_strict(input)?;
        Some(pos)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    Some(CircleShape { radius, position })
}

fn parse_ellipse_shape(input: &mut Parser<'_, '_>) -> Option<EllipseShape> {
    let radius_x = input
        .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
            try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    let radius_y = if radius_x.is_some() {
        input
            .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
                try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
    } else {
        None
    };
    let position = if input.try_parse(|i| i.expect_ident_matching("at")).is_ok() {
        let pos = parse_position_strict(input)?;
        Some(pos)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    Some(EllipseShape {
        radius_x,
        radius_y,
        position,
    })
}

fn parse_inset_shape(input: &mut Parser<'_, '_>) -> Option<InsetShape> {
    let mut insets: Vec<Length> = Vec::new();
    for _ in 0..4 {
        if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))
        }) {
            if lp.payload().is_nan() {
                return None;
            }
            insets.push(lp);
        } else {
            break;
        }
    }
    if insets.is_empty() {
        return None;
    }
    let border_radius = if input
        .try_parse(|i| i.expect_ident_matching("round"))
        .is_ok()
    {
        Some(parse_inset_border_radius(input)?)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    let (top, right, bottom, left) = match insets.len() {
        1 => (insets[0], insets[0], insets[0], insets[0]),
        2 => (insets[0], insets[1], insets[0], insets[1]),
        3 => (insets[0], insets[1], insets[2], insets[1]),
        4 => (insets[0], insets[1], insets[2], insets[3]),
        _ => unreachable!(),
    };
    Some(InsetShape {
        top,
        right,
        bottom,
        left,
        border_radius,
    })
}

fn parse_inset_border_radius(input: &mut Parser<'_, '_>) -> Option<InsetBorderRadius> {
    let mut horiz: Vec<Length> = Vec::new();
    for _ in 0..4 {
        if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            let l = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if l.payload() < 0.0 || l.payload().is_nan() {
                return Err(i.new_custom_error(()));
            }
            Ok(l)
        }) {
            horiz.push(lp);
        } else {
            break;
        }
    }
    if horiz.is_empty() {
        return None;
    }
    let vertical = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let mut vert: Vec<Length> = Vec::new();
        for _ in 0..4 {
            if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
                let l = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
                if l.payload() < 0.0 || l.payload().is_nan() {
                    return Err(i.new_custom_error(()));
                }
                Ok(l)
            }) {
                vert.push(lp);
            } else {
                break;
            }
        }
        if vert.is_empty() {
            return None;
        }
        Some(vert)
    } else {
        None
    };
    let horiz_expanded = expand_to_four(&horiz);
    let vert_expanded = vertical.as_ref().map(|v| expand_to_four(v));
    Some(InsetBorderRadius {
        horizontal: horiz_expanded,
        vertical: vert_expanded,
    })
}

fn expand_to_four(values: &[Length]) -> [Length; 4] {
    match values.len() {
        1 => [values[0]; 4],
        2 => [values[0], values[1], values[0], values[1]],
        3 => [values[0], values[1], values[2], values[1]],
        4 => [values[0], values[1], values[2], values[3]],
        _ => unreachable!(),
    }
}

fn parse_polygon_shape(input: &mut Parser<'_, '_>) -> Option<PolygonShape> {
    let mut fill_rule = FillRule::NonZero;
    let mut fill_rule_consumed = false;
    if let Ok(fr) = input.try_parse(|i| -> Result<FillRule, ParseError<'_, ()>> {
        parse_fill_rule(i).ok_or_else(|| i.new_custom_error(()))
    }) {
        fill_rule = fr;
        fill_rule_consumed = true;
    }
    if fill_rule_consumed {
        let _ = input.try_parse(|i| i.expect_comma()).ok();
    }
    let round = if input
        .try_parse(|i| i.expect_ident_matching("round"))
        .is_ok()
    {
        let len = parse_length_value(input, true)?;
        if len.payload() < 0.0 || len.payload().is_nan() {
            return None;
        }
        let _ = input.try_parse(|i| i.expect_comma()).ok();
        Some(len)
    } else {
        None
    };
    let mut points: Vec<(Length, Length)> = Vec::new();
    loop {
        let point = input.try_parse(|i| -> Result<(Length, Length), ParseError<'_, ()>> {
            let x = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if x.payload().is_nan() {
                return Err(i.new_custom_error(()));
            }
            let y = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if y.payload().is_nan() {
                return Err(i.new_custom_error(()));
            }
            Ok((x, y))
        });
        match point {
            Ok(p) => {
                points.push(p);
                let _ = input.try_parse(|i| i.expect_comma()).ok();
            }
            Err(_) => break,
        }
    }
    if points.is_empty() {
        return None;
    }
    if !input.is_exhausted() {
        return None;
    }
    Some(PolygonShape {
        fill_rule,
        round,
        points,
    })
}

fn parse_path_shape(input: &mut Parser<'_, '_>) -> Option<PathShape> {
    let mut fill_rule = FillRule::NonZero;
    let mut consumed = false;
    if let Ok(fr) = input.try_parse(|i| -> Result<FillRule, ParseError<'_, ()>> {
        parse_fill_rule(i).ok_or_else(|| i.new_custom_error(()))
    }) {
        fill_rule = fr;
        consumed = true;
    }
    if consumed {
        if input.try_parse(|i| i.expect_comma()).is_err() {
            return None;
        }
    } else {
        let _ = input.try_parse(|i| i.expect_comma()).ok();
    }
    let path_str = input.expect_string().ok()?.as_ref().to_string();
    if !input.is_exhausted() {
        return None;
    }
    Some(PathShape {
        fill_rule,
        path: path_str,
    })
}

/// `<number>` for the `transform` functions that take a bare number
/// (`matrix()`/`scale()`/`scaleX()`/`scaleY()`) — CSS Transforms Level 1
/// §9.1 places no range restriction on any of these, so — unlike
/// [`parse_filter_amount`], whose `>= 0.0` filter incidentally also
/// rejects NaN (`NaN >= 0.0` is `false` under IEEE 754) — this helper
/// cannot lean on a range check to catch the same hazard and must guard
/// explicitly. Same class of hazard as
/// [`PropertyValue::Opacity`]'s parser (`parse_opacity_value`'s
/// `!is_nan()` guard doc is canonical for the *mechanism*) — but the
/// *reason a guard is needed at all* here is `transform`-specific: this
/// crate's `Length`-typed box fields (`width`/`margin`/etc.) can go
/// unguarded because `raikiri-dom::layout::sanitize_finite` normalizes
/// NaN at the one sink that consumes them ([`Length`] doc's "型は層を
/// 表明しない" note describes that pipeline); `transform`'s
/// `f32`/[`Length`]/[`Angle`] payloads have no such downstream sink (no
/// paint-side consumer exists yet at all), so nothing else in this
/// crate's pipeline will ever normalize a NaN that slips past this
/// parser. `+Inf`/`-Inf` (ordinary magnitude overflow, a different hazard
/// class per `parse_opacity_value` doc's own distinction) are preserved —
/// no bound restricts a plain `<number>`.
///
/// `expect_number_stable` already corrects a huge-exponent, zero-mantissa
/// literal like `scale(0e999)` (module doc's "Numeric-token NaN
/// stabilization" section) before this function sees `n`, so in ordinary
/// use this `!is_nan()` check is defense-in-depth, not the primary
/// mechanism.
pub(crate) fn parse_transform_number(input: &mut Parser<'_, '_>) -> Option<f32> {
    let n = expect_number_stable(input).ok()?;
    (!n.is_nan()).then_some(n)
}

/// `<length-percentage>` for the `transform` functions that take one
/// (`translate()`/`translateX()`/`translateY()`) — same `!is_nan()`
/// rationale as [`parse_transform_number`], applied on top of
/// [`parse_length_value`]'s output instead of a bare `<number>`. Unlike
/// [`parse_non_negative_length`] (whose `>= 0.0` filter incidentally also
/// rejects NaN), `translate()`'s `<length-percentage>` has no sign
/// restriction, so there is no such incidental filter here — the guard
/// must be explicit, same shape as [`parse_transform_number`]'s own doc.
/// [`parse_length_value`] already routes through `next_numeric_stable`
/// (module doc above), so this is likewise defense-in-depth in ordinary
/// use.
pub(crate) fn parse_transform_length_percentage(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    (!length.payload().is_nan()).then_some(length)
}

/// `[<angle> | <zero>]` for `transform`'s `rotate()`/`skew()`/`skewX()`/
/// `skewY()` and `filter`'s `hue-rotate()` (both share this exact grammar,
/// CSS Transforms Level 1 §9.1 / CSS Filter Effects Level 1 §6.1 — neither
/// restricts `<angle>`'s range; `hue-rotate()`'s own text additionally
/// states implementations "must not normalize" the value, ruling out a
/// modulo-360 transform here). Wraps [`parse_angle`] (the shared
/// gradient-facing helper, `Result`-returning) with the same `!is_nan()`
/// guard [`parse_transform_number`] applies. `parse_angle` acquires its
/// token via `next_numeric_stable` (module doc above), which corrects a
/// huge-*exponent* literal like `rotate(0e999deg)` before `parse_angle`
/// ever computes degrees from it, so — same as the guards above — this is
/// defense-in-depth rather than the primary mechanism in ordinary use.
/// The guard is local so the shared gradient parser keeps its existing
/// behavior.
pub(crate) fn parse_angle_reject_nan(input: &mut Parser<'_, '_>) -> Option<Angle> {
    let angle = input.try_parse(parse_angle).ok()?;
    (!angle.0.is_nan()).then_some(angle)
}

fn parse_matrix_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let a = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let b = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let c = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let d = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let e = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let f = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(TransformFunction::Matrix([a, b, c, d, e, f]))
}

/// `translate(<length-percentage>, <length-percentage>?)` — 2nd argument
/// omitted defaults to `0` (CSS Transforms Level 1 §9.1 grammar's `?`
/// multiplier on the 2nd slot; the spec text names this default
/// explicitly in the 1-argument `translate()` case).
pub(crate) fn parse_translate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let tx = parse_transform_length_percentage(input).ok_or_else(|| input.new_custom_error(()))?;
    let ty = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_transform_length_percentage(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok(TransformFunction::Translate(tx, ty))
}

fn parse_translate_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_length_percentage(input)
        .map(TransformFunction::TranslateX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_translate_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_length_percentage(input)
        .map(TransformFunction::TranslateY)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `scale(<number>, <number>?)` — 2nd argument omitted **copies the 1st**
/// (CSS Transforms Level 1 §9.1's 1-argument `scale()` text: "the second
/// value defaults to the same value as the first"), unlike `translate()`'s
/// "defaults to 0" or `skew()`'s "defaults to 0deg" — three different
/// defaulting rules across the three 2-argument functions.
pub(crate) fn parse_scale_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let sx = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    let sy = input
        .try_parse(|i| -> Result<f32, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_transform_number(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(sx);
    Ok(TransformFunction::Scale(sx, sy))
}

fn parse_scale_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_number(input)
        .map(TransformFunction::ScaleX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_scale_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_number(input)
        .map(TransformFunction::ScaleY)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_rotate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::Rotate)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `skew([<angle> | <zero>], [<angle> | <zero>]?)` — 2nd argument omitted
/// defaults to `0deg` (CSS Transforms Level 1 §9.1's 1-argument `skew()`
/// text), the same "defaults to 0" shape as `translate()` (unlike
/// `scale()`'s "copies the 1st" — see `parse_scale_args` doc).
pub(crate) fn parse_skew_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let ax = parse_angle_reject_nan(input).ok_or_else(|| input.new_custom_error(()))?;
    let ay = input
        .try_parse(|i| -> Result<Angle, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_angle_reject_nan(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Angle(0.0));
    Ok(TransformFunction::Skew(ax, ay))
}

fn parse_skew_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::SkewX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_skew_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::SkewY)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `<transform-function>` (CSS Transforms Level 1 §9.1、[`TransformFunction`]
/// doc参照) の 1 function を function-token 名で dispatch する
/// ([`parse_gradient`] と同じ pattern)。3D function 名
/// (`translate3d`/`rotate3d`/`matrix3d`/`perspective` 等、§10) は
/// unrecognized name として `_` arm に落ち reject する
/// ([`TransformFunction`] doc の Non-goal 節参照)。
pub(crate) fn parse_transform_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "matrix" => input.parse_nested_block(parse_matrix_args),
        "translate" => input.parse_nested_block(parse_translate_args),
        "translatex" => input.parse_nested_block(parse_translate_x_args),
        "translatey" => input.parse_nested_block(parse_translate_y_args),
        "scale" => input.parse_nested_block(parse_scale_args),
        "scalex" => input.parse_nested_block(parse_scale_x_args),
        "scaley" => input.parse_nested_block(parse_scale_y_args),
        "rotate" => input.parse_nested_block(parse_rotate_args),
        "skew" => input.parse_nested_block(parse_skew_args),
        "skewx" => input.parse_nested_block(parse_skew_x_args),
        "skewy" => input.parse_nested_block(parse_skew_y_args),
        _ => Err(input.new_custom_error(())),
    }
}

/// `transform: none | <transform-list>` (CSS Transforms Level 1 §4、
/// `<transform-list> = <transform-function>[+]`
/// <https://www.w3.org/TR/css-transforms-1/#typedef-transform-list> —
/// **whitespace**-separated, not comma-separated, one or more) を parse
/// する。[`parse_text_decoration_line`]'s `||` loop と同じ「`try_parse` が
/// 失敗するまで繰り返す」shape — cssparser の `Parser::next`/`try_parse` は
/// token 間の whitespace を自動 skip するため、明示的な separator handling
/// は不要。
pub(super) fn parse_transform(input: &mut Parser<'_, '_>) -> Option<Vec<TransformFunction>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    while let Ok(function) = input.try_parse(parse_transform_function) {
        functions.push(function);
    }
    (!functions.is_empty()).then_some(functions)
}

/// `<number-percentage>` for the 7 `filter` amount functions
/// (`brightness()`/`contrast()`/`grayscale()`/`invert()`/`opacity()`/
/// `saturate()`/`sepia()`, CSS Filter Effects Level 1 §6.1). Each states
/// "Negative values are not allowed" with no opacity-property-style
/// specified/computed split (`filter`'s own Computed value is "as
/// specified", [`FilterFunction`] doc's "Range restriction is reject, not
/// clamp" section) — so this crate rejects a negative parse outright
/// (`None`), matching [`parse_nonneg_finite_number`](super::layout::parse_nonneg_finite_number)'s (flex-grow/
/// flex-shrink) reject-at-parse precedent rather than
/// [`parse_opacity_value`]'s preserve-then-clamp one.
///
/// No separate `!is_nan()` guard is needed: `v >= 0.0` is `false` for NaN
/// under IEEE 754 comparison semantics, so the same range check that
/// rejects an ordinary negative value would incidentally also reject a
/// NaN parse — unlike [`parse_transform_number`] (whose callers have no
/// range restriction to lean on and must guard explicitly), this helper's
/// range restriction alone would do the job. In practice a huge-exponent
/// literal like `brightness(0e999)` no longer reaches this check as NaN
/// at all — `expect_number_stable`/`expect_percentage_stable` (module
/// doc's "Numeric-token NaN stabilization" section) already correct it to
/// `0.0` at acquisition, so `v >= 0.0` accepts it normally instead of
/// incidentally rejecting it.
pub(crate) fn parse_filter_amount<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    let v = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        pct
    } else {
        expect_number_stable(input)?
    };
    if v >= 0.0 {
        Ok(v)
    } else {
        Err(input.new_custom_error(()))
    }
}

fn parse_blur_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    let length = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_non_negative_length(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok(FilterFunction::Blur(length))
}

fn parse_brightness_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Brightness(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_contrast_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Contrast(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_grayscale_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Grayscale(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_hue_rotate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let angle = input
        .try_parse(|i| -> Result<Angle, ParseError<'_, ()>> {
            parse_angle_reject_nan(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Angle(0.0));
    Ok(FilterFunction::HueRotate(angle))
}

fn parse_invert_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Invert(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_filter_opacity_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Opacity(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_saturate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Saturate(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_sepia_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Sepia(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

/// `drop-shadow(<color>? && <length>{2,3})` — CSS Filter Effects Level 1
/// §6.1: "Values are interpreted as for box-shadow but with the optional
/// 3rd `<length>` value being the standard deviation instead of blur
/// radius" — grammar-identical to `text-shadow`'s own `<shadow>` syntax
/// (no spread, no inset), so [`parse_text_shadow_item`] is reused verbatim
/// ([`FilterFunction::DropShadow`] doc参照). [`parse_text_shadow_lengths`]'s
/// offset-x/offset-y already go through [`parse_shadow_length_reject_nan`]'s
/// `!is_nan()` guard (see that function's doc), so this reuse inherits the
/// guard automatically.
pub(crate) fn parse_drop_shadow_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let item = parse_text_shadow_item(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(FilterFunction::DropShadow(item))
}

/// `<filter-function>` (CSS Filter Effects Level 1 §6、[`FilterFunction`]
/// doc参照) の 1 function を function-token 名で dispatch する — `<url>`
/// alternative は含まない ([`parse_filter`] が別途試す、`url` という
/// function 名はここでは未知 name として reject される)。
fn parse_filter_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "blur" => input.parse_nested_block(parse_blur_args),
        "brightness" => input.parse_nested_block(parse_brightness_args),
        "contrast" => input.parse_nested_block(parse_contrast_args),
        "grayscale" => input.parse_nested_block(parse_grayscale_args),
        "hue-rotate" => input.parse_nested_block(parse_hue_rotate_args),
        "invert" => input.parse_nested_block(parse_invert_args),
        "opacity" => input.parse_nested_block(parse_filter_opacity_args),
        "saturate" => input.parse_nested_block(parse_saturate_args),
        "sepia" => input.parse_nested_block(parse_sepia_args),
        "drop-shadow" => input.parse_nested_block(parse_drop_shadow_args),
        _ => Err(input.new_custom_error(())),
    }
}

/// `filter: none | <filter-value-list>` (CSS Filter Effects Level 1 §5、
/// `<filter-value-list> = [ <filter-function> | <url> ]+` —
/// **whitespace**-separated, not comma-separated, one or more) を parse
/// する。[`parse_transform`] と同じ loop shape だが、各要素で
/// [`parse_filter_function`] (named function) を先に試し、失敗したら
/// [`parse_url_value`] (`<url>` alternative) を試す 2-way fallback
/// ([`parse_background_image`] の gradient-then-url 順序と同じ理由 —
/// 互いに排他的な token shape なので試行順は結果を左右しない)。
pub(super) fn parse_filter(input: &mut Parser<'_, '_>) -> Option<Vec<FilterFunction>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    loop {
        if let Ok(function) = input.try_parse(parse_filter_function) {
            functions.push(function);
            continue;
        }
        if let Ok(url) = input.try_parse(|i| -> Result<String, ParseError<'_, ()>> {
            parse_url_value(i).ok_or_else(|| i.new_custom_error(()))
        }) {
            functions.push(FilterFunction::Url(url));
            continue;
        }
        break;
    }
    (!functions.is_empty()).then_some(functions)
}

/// `<gradient>` (CSS Images 4 §3 — [`Gradient`] doc参照) の 6 function 名を
/// dispatch する。function token の名前を見てから対応する
/// `parse_*_gradient_body` を `parse_nested_block` で呼ぶ — `color-mix()`
/// 等の他 function dispatch ([`parse_color_float`] の match) と同じ形。
fn parse_gradient<'i>(input: &mut Parser<'i, '_>) -> Result<Gradient, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "linear-gradient" => input
            .parse_nested_block(|i| parse_linear_gradient_body(i, false))
            .map(Gradient::Linear),
        "repeating-linear-gradient" => input
            .parse_nested_block(|i| parse_linear_gradient_body(i, true))
            .map(Gradient::Linear),
        "radial-gradient" => input
            .parse_nested_block(|i| parse_radial_gradient_body(i, false))
            .map(Gradient::Radial),
        "repeating-radial-gradient" => input
            .parse_nested_block(|i| parse_radial_gradient_body(i, true))
            .map(Gradient::Radial),
        "conic-gradient" => input
            .parse_nested_block(|i| parse_conic_gradient_body(i, false))
            .map(Gradient::Conic),
        "repeating-conic-gradient" => input
            .parse_nested_block(|i| parse_conic_gradient_body(i, true))
            .map(Gradient::Conic),
        _ => Err(input.new_custom_error(())),
    }
}

/// `<angle> | <zero>` (CSS Values 4 §7.1、[`Angle`] doc参照)。bare `0` のみ
/// unitless を許す — CSS Values 4 §7.1 が明記する通り、`<angle>` 自体は
/// 一般には unitless zero を許さない ("For legacy reasons, some uses of
/// `<angle>` allow a bare 0 to mean 0deg. This is not true in general")。
/// この crate が呼び出し元 (`linear-gradient()` の `[ <angle> | <zero> | to
/// <side-or-corner> ]`、`conic-gradient()` の `from [ <angle> | <zero> ]`、
/// いずれも CSS Images 4 §3) の grammar に明示的な `<zero>` alternative を
/// 持つ「legacy な用法」に該当するため、bare `0` を受理する
/// (`<length-percentage>` の unitless-zero — [`parse_length_value`]の
/// "Unitless zero" 節 — とは別の、angle 固有の根拠)。単位変換
/// (grad/rad/turn → deg) の overflow saturation は同関数の "Percentage
/// overflow" 節と同じ方針 (`is_infinite()` の場合のみ符号を保持して
/// `f32::MAX` へ寄せる、`NaN` は無変換)。token 取得は `next_numeric_stable`
/// 経由 (module doc 冒頭「Numeric-token NaN stabilization」節参照) — 通常の
/// parse では `value` に `0e999deg` 由来の `NaN` が届くことはもう無い。
fn parse_angle<'i>(input: &mut Parser<'i, '_>) -> Result<Angle, ParseError<'i, ()>> {
    match next_numeric_stable(input)? {
        Token::Number { value, .. } => {
            if value == 0.0 {
                Ok(Angle(0.0))
            } else {
                Err(input.new_custom_error(()))
            }
        }
        Token::Dimension { value, unit, .. } => {
            let degrees = match unit.to_ascii_lowercase().as_str() {
                "deg" => value,
                "grad" => value * 0.9,
                "rad" => value.to_degrees(),
                "turn" => value * 360.0,
                _ => return Err(input.new_custom_error(())),
            };
            let degrees = if degrees.is_infinite() {
                f32::MAX.copysign(degrees)
            } else {
                degrees
            };
            Ok(Angle(degrees))
        }
        token => Err(input.new_unexpected_token_error(token)),
    }
}

/// `<angle-percentage>` ([`AnglePercentage`] doc参照) — `conic-gradient()`
/// の angular color stop position。
fn parse_angle_percentage<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AnglePercentage, ParseError<'i, ()>> {
    if let Ok(percent) = input.try_parse(parse_percent_number) {
        return Ok(AnglePercentage::Percent(percent));
    }
    parse_angle(input).map(AnglePercentage::Angle)
}

/// `<percentage>` token を authored number (`50%` → `50.0`) へ変換する —
/// [`parse_length_value`] の `Token::Percentage` arm と同じ overflow
/// saturation 方針 (`unit_value * 100.0` の逆変換が `±Inf` になった場合のみ
/// 符号を保持して `f32::MAX` へ寄せる、同関数の "Percentage overflow" 節
/// 参照)。
fn parse_percent_number<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    let unit_value = expect_percentage_stable(input)?;
    let percent = unit_value * 100.0;
    Ok(if percent.is_infinite() {
        f32::MAX.copysign(percent)
    } else {
        percent
    })
}

/// `in <color-space> <hue-interpolation-method>?` ([`GradientColorInterpolation`]
/// doc参照) — parse + validation は `parse_color_mix_function`と共有する
/// [`parse_color_interpolation_method`] に委譲し、ここでは結果 tuple を
/// [`GradientColorInterpolation`] へ組み立てるだけ。
pub(crate) fn parse_gradient_color_interpolation<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GradientColorInterpolation, ParseError<'i, ()>> {
    let (color_space, hue_method) = parse_color_interpolation_method(input, false)?;
    Ok(GradientColorInterpolation {
        color_space,
        hue_method,
    })
}

/// [`GradientColorInterpolation`] の spec-mandated default — `in ...` 節
/// 省略時、CSS Images 4 §3.5.2 "Coloring the Gradient Line" "the color
/// space used for gradient interpolation is the default interpolation
/// color space, Oklab"。
fn default_gradient_color_interpolation() -> GradientColorInterpolation {
    GradientColorInterpolation {
        color_space: MixColorSpace::Oklab,
        hue_method: HueInterpolationMethod::Shorter,
    }
}

/// `at <position>` ([`CssPosition`] doc参照) — `radial-gradient()` /
/// `conic-gradient()` 共通。
fn parse_at_position<'i>(input: &mut Parser<'i, '_>) -> Result<CssPosition, ParseError<'i, ()>> {
    input.expect_ident_matching("at")?;
    parse_bg_position(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<position>` の spec-mandated default (`center`、[`css_position_center`]
/// を両軸に適用)。
fn default_center_position() -> CssPosition {
    CssPosition {
        horizontal: css_position_center(),
        vertical: css_position_center(),
    }
}

/// `<color>` を持つ gradient stop の color ([`GradientStopColor`] doc参照)
/// — `currentcolor` keyword を先取りしてから [`parse_color`] へ委譲する
/// ([`parse_text_shadow_color`] と同じ shape)。
fn parse_gradient_stop_color(input: &mut Parser<'_, '_>) -> Option<GradientStopColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(GradientStopColor::CurrentColor);
    }
    parse_color(input).map(GradientStopColor::Resolved)
}

/// `<linear-color-stop> = <color> <length-percentage>?`
/// ([`GradientColorStop`] doc参照)。
fn parse_gradient_color_stop<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GradientColorStop, ParseError<'i, ()>> {
    let color = parse_gradient_stop_color(input).ok_or_else(|| input.new_custom_error(()))?;
    let position = input.try_parse(parse_length_percentage_res).ok();
    Ok(GradientColorStop { color, position })
}

/// `<color-stop-list>` ([`GradientColorStop`] doc の scope carving 節 —
/// hint 無し、2 個以上必須)。`linear-gradient()`/`radial-gradient()` 共通。
fn parse_gradient_color_stop_list<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<GradientColorStop>, ParseError<'i, ()>> {
    let stops = input.parse_comma_separated(parse_gradient_color_stop)?;
    if stops.len() < 2 {
        return Err(input.new_custom_error(()));
    }
    Ok(stops)
}

/// `<angular-color-stop> = <color> <color-stop-angle>?` の 1-value 版
/// ([`AngularColorStop`] doc参照)。
fn parse_angular_color_stop<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AngularColorStop, ParseError<'i, ()>> {
    let color = parse_gradient_stop_color(input).ok_or_else(|| input.new_custom_error(()))?;
    let position = input.try_parse(parse_angle_percentage).ok();
    Ok(AngularColorStop { color, position })
}

/// `<angular-color-stop-list>` — [`parse_gradient_color_stop_list`]の
/// conic 版 (同じ scope carving、同じ 2 個以上 minimum)。
fn parse_angular_color_stop_list<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<AngularColorStop>, ParseError<'i, ()>> {
    let stops = input.parse_comma_separated(parse_angular_color_stop)?;
    if stops.len() < 2 {
        return Err(input.new_custom_error(()));
    }
    Ok(stops)
}

/// [`LinearGradientDirection`]の spec-mandated default (`to bottom`、CSS
/// Images 4 §3.1 "If the first argument to the function is omitted, it
/// defaults to to bottom")。
fn default_linear_gradient_direction() -> LinearGradientDirection {
    LinearGradientDirection::Side(SideOrCorner {
        horizontal: None,
        vertical: Some(VerticalSide::Bottom),
    })
}

/// `to <side-or-corner>` の `to` 抜きの部分、または `<angle>` — どちらか
/// ([`LinearGradientDirection`] doc の 2 alternative)。
fn parse_linear_gradient_direction<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LinearGradientDirection, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("to")).is_ok() {
        return parse_side_or_corner(input).map(LinearGradientDirection::Side);
    }
    parse_angle(input).map(LinearGradientDirection::Angle)
}

/// `<side-or-corner> = [left | right] || [top | bottom]`
/// ([`SideOrCorner`] doc参照) — [`parse_outline`](super::box_model::parse_outline)等と同じ any-order loop。
fn parse_side_or_corner<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SideOrCorner, ParseError<'i, ()>> {
    let mut horizontal: Option<HorizontalSide> = None;
    let mut vertical: Option<VerticalSide> = None;
    loop {
        if horizontal.is_none()
            && let Ok(value) = input.try_parse(parse_horizontal_side)
        {
            horizontal = Some(value);
            continue;
        }
        if vertical.is_none()
            && let Ok(value) = input.try_parse(parse_vertical_side)
        {
            vertical = Some(value);
            continue;
        }
        break;
    }
    if horizontal.is_none() && vertical.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok(SideOrCorner {
        horizontal,
        vertical,
    })
}

fn parse_horizontal_side<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<HorizontalSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "left" => Ok(HorizontalSide::Left),
        "right" => Ok(HorizontalSide::Right),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_vertical_side<'i>(input: &mut Parser<'i, '_>) -> Result<VerticalSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "top" => Ok(VerticalSide::Top),
        "bottom" => Ok(VerticalSide::Bottom),
        _ => Err(input.new_custom_error(())),
    }
}

/// `linear-gradient()`/`repeating-linear-gradient()` の nested block body
/// ([`LinearGradient`] doc の grammar 参照)。`direction`/`interpolation` は
/// `||` (any order, both optional) — 見つかった場合のみ、続く
/// `<color-stop-list>` の前に comma が要る (grammar の trailing `,` は
/// `[...]?` group の**外**にあるので、group が空なら comma も現れない —
/// `linear-gradient(red, blue)` に direction/interpolation が無いのと同じ
/// 理由)。
fn parse_linear_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<LinearGradient, ParseError<'i, ()>> {
    let mut direction: Option<LinearGradientDirection> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if direction.is_none()
            && let Ok(value) = input.try_parse(parse_linear_gradient_direction)
        {
            direction = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    if direction.is_some() || interpolation.is_some() {
        input.expect_comma()?;
    }
    let stops = parse_gradient_color_stop_list(input)?;
    Ok(LinearGradient {
        repeating,
        direction: direction.unwrap_or_else(default_linear_gradient_direction),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

fn parse_radial_shape_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialShape, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "circle" => Ok(RadialShape::Circle),
        "ellipse" => Ok(RadialShape::Ellipse),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_radial_extent<'i>(input: &mut Parser<'i, '_>) -> Result<RadialExtent, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "closest-side" => Ok(RadialExtent::ClosestSide),
        "closest-corner" => Ok(RadialExtent::ClosestCorner),
        "farthest-side" => Ok(RadialExtent::FarthestSide),
        "farthest-corner" => Ok(RadialExtent::FarthestCorner),
        _ => Err(input.new_custom_error(())),
    }
}

/// [`RadialSize`]の authored form — [`resolve_radial_shape_and_size`] が
/// (省略された `shape` と合わせて) 最終的な `(RadialShape, RadialSize)` へ
/// 解決する前の中間表現。`Circle`/`Ellipse` は [`RadialSize`]と同じ意味だが
/// まだ `shape` との整合性を確認していない。
enum RadialSizeAuthored {
    Extent(RadialExtent),
    Circle(Length),
    Ellipse(Length, Length),
}

/// [`parse_radial_shape_size_position_group`] の戻り値 shape — clippy
/// `type_complexity` を避けるための alias (意味論的な新型ではない)。
type RadialShapeSizePositionGroup = (
    Option<RadialShape>,
    Option<RadialSizeAuthored>,
    Option<CssPosition>,
);

/// `<radial-size>` (CSS Images 3 §3.2.1 baseline grammar、[`RadialSize`]
/// doc参照)。2 つの `<length-percentage [0,∞]>` (ellipse form) を先に試す —
/// 単一 token しか無い入力では 2 個目の parse が失敗して丸ごと rewind し、
/// 単一 `<length [0,∞]>` (circle form) へ自然に fall back する
/// ([`parse_bg_position`]の alternative 順序 doc と同じ「安全な rewind」
/// 構造)。
fn parse_radial_size<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialSizeAuthored, ParseError<'i, ()>> {
    if let Ok(extent) = input.try_parse(parse_radial_extent) {
        return Ok(RadialSizeAuthored::Extent(extent));
    }
    if let Ok((a, b)) = input.try_parse(parse_two_non_negative_length_percentages) {
        return Ok(RadialSizeAuthored::Ellipse(a, b));
    }
    let length = parse_non_negative_length(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(RadialSizeAuthored::Circle(length))
}

fn parse_two_non_negative_length_percentages<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length), ParseError<'i, ()>> {
    let a = parse_non_negative_length_percentage_res(input)?;
    let b = parse_non_negative_length_percentage_res(input)?;
    Ok((a, b))
}

/// 省略された `shape`/`size` を CSS Images 3 §3.2.1 の規則で解決する —
/// [`RadialShape`]/[`RadialSize`]のペア doc参照。無効な組み合わせ
/// (`circle` + ellipse-only size、`ellipse` + circle-only size) は `None`。
fn resolve_radial_shape_and_size(
    shape: Option<RadialShape>,
    size: Option<RadialSizeAuthored>,
) -> Option<(RadialShape, RadialSize)> {
    match (shape, size) {
        (Some(RadialShape::Circle), Some(RadialSizeAuthored::Ellipse(_, _)))
        | (Some(RadialShape::Ellipse), Some(RadialSizeAuthored::Circle(_))) => None,
        (Some(RadialShape::Circle), Some(RadialSizeAuthored::Circle(l))) => {
            Some((RadialShape::Circle, RadialSize::Circle(l)))
        }
        (Some(RadialShape::Ellipse), Some(RadialSizeAuthored::Ellipse(a, b))) => {
            Some((RadialShape::Ellipse, RadialSize::Ellipse(a, b)))
        }
        (Some(shape), Some(RadialSizeAuthored::Extent(extent))) => {
            Some((shape, RadialSize::Extent(extent)))
        }
        (Some(shape), None) => Some((shape, RadialSize::Extent(RadialExtent::FarthestCorner))),
        (None, Some(RadialSizeAuthored::Circle(l))) => {
            Some((RadialShape::Circle, RadialSize::Circle(l)))
        }
        (None, Some(RadialSizeAuthored::Ellipse(a, b))) => {
            Some((RadialShape::Ellipse, RadialSize::Ellipse(a, b)))
        }
        (None, Some(RadialSizeAuthored::Extent(extent))) => {
            Some((RadialShape::Ellipse, RadialSize::Extent(extent)))
        }
        (None, None) => Some((
            RadialShape::Ellipse,
            RadialSize::Extent(RadialExtent::FarthestCorner),
        )),
    }
}

/// `[ <radial-shape> || <radial-size> ]? [ at <position> ]?` — spec の
/// juxtaposition (space 区切り) が示す通り、`shape`/`size` は互いに
/// any-order (内側 `||`) だが、`at <position>` はこのグループ全体の
/// **後**にしか現れない (順序固定)。`shape`/`size`/`position` のいずれも
/// 無ければ `Err` を返す — [`parse_radial_gradient_body`]側の outer `||`
/// loop が「このグループを 1 要素として `<color-interpolation-method>`
/// と任意順に読む」ために、空マッチと「何も無かった」を区別する必要が
/// あるため (`try_parse` が空マッチを毎回成功として返すと、outer loop の
/// 2 巡目以降でこのグループを再試行できなくなる)。
fn parse_radial_shape_size_position_group<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialShapeSizePositionGroup, ParseError<'i, ()>> {
    let mut shape: Option<RadialShape> = None;
    let mut size: Option<RadialSizeAuthored> = None;
    loop {
        if shape.is_none()
            && let Ok(value) = input.try_parse(parse_radial_shape_keyword)
        {
            shape = Some(value);
            continue;
        }
        if size.is_none()
            && let Ok(value) = input.try_parse(parse_radial_size)
        {
            size = Some(value);
            continue;
        }
        break;
    }
    let position = input.try_parse(parse_at_position).ok();
    if shape.is_none() && size.is_none() && position.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok((shape, size, position))
}

/// `radial-gradient()`/`repeating-radial-gradient()` の nested block body
/// ([`RadialGradient`] doc の grammar 参照)。Grammar `[ [ [ <radial-shape>
/// || <radial-size> ]? [ at <position> ]? ] || <color-interpolation-method>
/// ]?` の外側 `||` (shape/size/position グループと
/// `<color-interpolation-method>` の間) を any-order loop で読む —
/// グループ内部の順序制約 ([`parse_radial_shape_size_position_group`]
/// doc参照) は [`parse_linear_gradient_body`]の 2-slot 構造と異なりグループ
/// 自体が 1 slot になっている点に注意 (`shape`/`size`/`position` を outer
/// loop の独立した slot にすると `at center circle` のような spec-invalid
/// な順序 — position が shape/size より前 — まで受理してしまう)。
fn parse_radial_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<RadialGradient, ParseError<'i, ()>> {
    let mut group: Option<RadialShapeSizePositionGroup> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if group.is_none()
            && let Ok(value) = input.try_parse(parse_radial_shape_size_position_group)
        {
            group = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    let any = group.is_some() || interpolation.is_some();
    if any {
        input.expect_comma()?;
    }
    let stops = parse_gradient_color_stop_list(input)?;
    let (shape, size, position) = group.unwrap_or((None, None, None));
    let (resolved_shape, resolved_size) =
        resolve_radial_shape_and_size(shape, size).ok_or_else(|| input.new_custom_error(()))?;
    Ok(RadialGradient {
        repeating,
        shape: resolved_shape,
        size: resolved_size,
        position: position.unwrap_or_else(default_center_position),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

fn parse_conic_from_angle<'i>(input: &mut Parser<'i, '_>) -> Result<Angle, ParseError<'i, ()>> {
    input.expect_ident_matching("from")?;
    parse_angle(input)
}

/// `[ from [ <angle> | <zero> ] ]? [ at <position> ]?` —
/// [`parse_radial_shape_size_position_group`]の conic 版。spec の
/// juxtaposition により `from <angle>` は `at <position>` より必ず先
/// (こちらは shape/size 側と違い、先頭要素自体が単一 component なので
/// 内側に `||` は無い)。空マッチと区別するための `Err` fallback も同型。
fn parse_conic_from_position_group<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Option<Angle>, Option<CssPosition>), ParseError<'i, ()>> {
    let angle = input.try_parse(parse_conic_from_angle).ok();
    let position = input.try_parse(parse_at_position).ok();
    if angle.is_none() && position.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok((angle, position))
}

/// `conic-gradient()`/`repeating-conic-gradient()` の nested block body
/// ([`ConicGradient`] doc の grammar 参照)。外側 `||` (`from`/`at`
/// グループと `<color-interpolation-method>` の間) を any-order loop で
/// 読む — [`parse_radial_gradient_body`]と同じ「グループを 1 slot として
/// 扱う」構造 (`at center from 45deg` のような spec-invalid な逆順を
/// 拒否するのに必要、[`parse_conic_from_position_group`] doc参照)。
fn parse_conic_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<ConicGradient, ParseError<'i, ()>> {
    let mut group: Option<(Option<Angle>, Option<CssPosition>)> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if group.is_none()
            && let Ok(value) = input.try_parse(parse_conic_from_position_group)
        {
            group = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    let any = group.is_some() || interpolation.is_some();
    if any {
        input.expect_comma()?;
    }
    let stops = parse_angular_color_stop_list(input)?;
    let (angle, position) = group.unwrap_or((None, None));
    Ok(ConicGradient {
        repeating,
        angle: angle.unwrap_or(Angle(0.0)),
        position: position.unwrap_or_else(default_center_position),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

/// `<repeat-style>` の 1 keyword を parse する ([`BackgroundRepeatKeyword`]
/// doc の grammar 参照)。`repeat-x`/`repeat-y` はここでは扱わない
/// ([`parse_background_repeat`] が別途 top-level alternative として処理)。
fn parse_background_repeat_keyword(input: &mut Parser<'_, '_>) -> Option<BackgroundRepeatKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "repeat" => Some(BackgroundRepeatKeyword::Repeat),
        "space" => Some(BackgroundRepeatKeyword::Space),
        "round" => Some(BackgroundRepeatKeyword::Round),
        "no-repeat" => Some(BackgroundRepeatKeyword::NoRepeat),
        _ => None,
    }
}

fn parse_background_repeat_keyword_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BackgroundRepeatKeyword, ParseError<'i, ()>> {
    parse_background_repeat_keyword(input).ok_or_else(|| input.new_custom_error(()))
}

/// `background-repeat: <repeat-style>` を parse する ([`BackgroundRepeat`]
/// doc の grammar 参照: `repeat-x | repeat-y | [repeat | space | round |
/// no-repeat]{1,2}`)。
///
/// `repeat-x`/`repeat-y` は 2-keyword form の shorthand として先に試す
/// (spec computed value: `repeat-x` = `repeat no-repeat`、`repeat-y` =
/// `no-repeat repeat`)。1 keyword のみ指定時は両軸に同じ値を適用する
/// (`repeat` = `repeat repeat` 等)。
pub(crate) fn parse_background_repeat(input: &mut Parser<'_, '_>) -> Option<BackgroundRepeat> {
    if input
        .try_parse(|i| i.expect_ident_matching("repeat-x"))
        .is_ok()
    {
        return Some(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("repeat-y"))
        .is_ok()
    {
        return Some(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::Repeat,
        });
    }
    let first = parse_background_repeat_keyword(input)?;
    let second = input.try_parse(parse_background_repeat_keyword_res).ok();
    Some(match second {
        None => BackgroundRepeat { x: first, y: first },
        Some(second) => BackgroundRepeat {
            x: first,
            y: second,
        },
    })
}

/// `background-attachment: <attachment>` を parse する
/// ([`BackgroundAttachment`] doc の grammar 参照: `scroll | fixed | local`)。
pub(super) fn parse_background_attachment(
    input: &mut Parser<'_, '_>,
) -> Option<BackgroundAttachment> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "scroll" => Some(BackgroundAttachment::Scroll),
        "fixed" => Some(BackgroundAttachment::Fixed),
        "local" => Some(BackgroundAttachment::Local),
        _ => None,
    }
}

/// `object-fit: <fit>` を parse する ([`ObjectFit`] doc の grammar 参照:
/// `fill | contain | cover | none | scale-down`)。
pub(super) fn parse_object_fit(input: &mut Parser<'_, '_>) -> Option<ObjectFit> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "fill" => Some(ObjectFit::Fill),
        "contain" => Some(ObjectFit::Contain),
        "cover" => Some(ObjectFit::Cover),
        "none" => Some(ObjectFit::None),
        "scale-down" => Some(ObjectFit::ScaleDown),
        _ => None,
    }
}

/// `isolation: <isolation-mode>` を parse する ([`Isolation`] doc の grammar
/// 参照: `auto | isolate`)。
pub(super) fn parse_isolation(input: &mut Parser<'_, '_>) -> Option<Isolation> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(Isolation::Auto),
        "isolate" => Some(Isolation::Isolate),
        _ => None,
    }
}

/// `mix-blend-mode: <blend-mode>` を parse する ([`MixBlendMode`] doc の
/// grammar 参照: 16 keyword)。
pub(super) fn parse_mix_blend_mode(input: &mut Parser<'_, '_>) -> Option<MixBlendMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(MixBlendMode::Normal),
        "multiply" => Some(MixBlendMode::Multiply),
        "screen" => Some(MixBlendMode::Screen),
        "overlay" => Some(MixBlendMode::Overlay),
        "darken" => Some(MixBlendMode::Darken),
        "lighten" => Some(MixBlendMode::Lighten),
        "color-dodge" => Some(MixBlendMode::ColorDodge),
        "color-burn" => Some(MixBlendMode::ColorBurn),
        "hard-light" => Some(MixBlendMode::HardLight),
        "soft-light" => Some(MixBlendMode::SoftLight),
        "difference" => Some(MixBlendMode::Difference),
        "exclusion" => Some(MixBlendMode::Exclusion),
        "hue" => Some(MixBlendMode::Hue),
        "saturation" => Some(MixBlendMode::Saturation),
        "color" => Some(MixBlendMode::Color),
        "luminosity" => Some(MixBlendMode::Luminosity),
        _ => None,
    }
}

/// `<visual-box>` を parse する ([`VisualBox`] doc の grammar 参照:
/// `border-box | padding-box | content-box`)。`background-clip` /
/// `background-origin` 共有 (initial value の違いは呼び出し元ではなく
/// `specified.rs`/`computed.rs` 側で扱う)。
pub(super) fn parse_visual_box(input: &mut Parser<'_, '_>) -> Option<VisualBox> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "border-box" => Some(VisualBox::BorderBox),
        "padding-box" => Some(VisualBox::PaddingBox),
        "content-box" => Some(VisualBox::ContentBox),
        "border-area" => Some(VisualBox::BorderArea),
        "text" => Some(VisualBox::Text),
        _ => None,
    }
}

/// `<bg-size>` の 1 軸分 — `<length-percentage [0,∞]> | auto`
/// ([`parse_width`](super::box_model::parse_width) と同じ non-negative enforcement pattern)。
fn parse_background_size_axis(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length.payload() >= 0.0).then_some(LengthOrAuto::Length(length))
}

fn parse_background_size_axis_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthOrAuto, ParseError<'i, ()>> {
    parse_background_size_axis(input).ok_or_else(|| input.new_custom_error(()))
}

/// `background-size: <bg-size>` を parse する ([`BackgroundSize`] doc の
/// grammar 参照: `[ <length-percentage [0,∞]> | auto ]{1,2} | cover |
/// contain`)。
///
/// `cover`/`contain` は keyword 全体を占有するため axis run より先に試す。
/// 2 個目の axis が省略された場合は **`auto`** (spec verbatim: "If only
/// one value is given the second is assumed to be auto.") — 1 個目の
/// 値を複製する [`parse_border_radius`](super::box_model::parse_border_radius) 系の fill 規則とは異なるので
/// 流用しない。
pub(crate) fn parse_background_size(input: &mut Parser<'_, '_>) -> Option<BackgroundSize> {
    if input
        .try_parse(|i| i.expect_ident_matching("cover"))
        .is_ok()
    {
        return Some(BackgroundSize::Cover);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("contain"))
        .is_ok()
    {
        return Some(BackgroundSize::Contain);
    }
    let width = parse_background_size_axis(input)?;
    let height = input
        .try_parse(parse_background_size_axis_res)
        .unwrap_or(LengthOrAuto::Auto);
    Some(BackgroundSize::Explicit { width, height })
}

/// `<bg-position> [ / <bg-size> ]?` — the `background` shorthand's
/// position+size unit ([`BackgroundShorthand`] doc's grammar section).
/// Unlike the shorthand's other `||` components, position and size are
/// **not** independently orderable — a size may only follow a position,
/// separated by a literal `/` (same fixed-pair shape as
/// [`parse_grid_line_shorthand`](super::layout::parse_grid_line_shorthand)'s `<grid-line> [ / <grid-line> ]?`).
///
/// [`parse_bg_position`]'s 3 internal alternatives (`branch3`/`branch2`/
/// `branch1`, see that function's doc) each consume exactly their own
/// production and stop — they never overrun into tokens that belong to a
/// later shorthand component. That is what lets this function's caller
/// ([`parse_background_shorthand`]) safely hand any leftover tokens back to
/// its own any-order loop instead of treating them as part of the position.
pub(crate) fn parse_background_position_and_size(
    input: &mut Parser<'_, '_>,
) -> Option<(CssPosition, Option<BackgroundSize>)> {
    let position = parse_bg_position(input)?;
    let size = input
        .try_parse(|i| -> Result<BackgroundSize, ParseError<'_, ()>> {
            i.expect_delim('/')?;
            parse_background_size(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    Some((position, size))
}

/// `background` shorthand を parse する ([`BackgroundShorthand`] doc の
/// grammar 節参照)。
///
/// # `||` (any-order) grammar semantics
///
/// [`parse_border_shorthand`](super::box_model::parse_border_shorthand) と同じ loop 構造: 各 unfilled slot を
/// `try_parse` で順に試し、成功したら slot を埋めて loop 先頭に戻る。全 slot
/// 満了、または未 match token に当たったら break — leftover は呼び出し元
/// (`rule.rs` の declaration parser) の `expect_exhausted` が丸ごと drop する
/// ([`BackgroundShorthand`] doc の「単一 layer のみ対応」節が、これを使って
/// comma-separated 複数 layer を reject する仕組みを説明している)。
///
/// 6 slot のうち `visual_boxes` だけ最大 2 回一致しうる — `<visual-box>` が
/// grammar 上 2 回独立した `||` alternative として現れるため
/// ([`BackgroundShorthand`] doc 参照)。`position_and_size` は
/// [`parse_background_position_and_size`] 経由で 1 slot として扱う (spec の
/// `<bg-position> [ / <bg-size> ]?` を単一の `||` alternative として)。
///
/// 各 slot の token 集合は互いに素 (image は `none`/`url()`/gradient
/// function、position は方向 keyword/length、repeat-style/attachment/
/// visual-box はそれぞれ固有 keyword 集合、color は named color/hex/function)
/// なので、slot を試す順序自体は結果を左右しない —
/// [`parse_border_shorthand`](super::box_model::parse_border_shorthand) doc の同旨コメント参照。
///
/// # At least 1 component required
///
/// spec CSS Values 4 §2.2 `||` semantics: "one or more of them must occur,
/// in any order." — 0 component (空 `background:` や未知 keyword のみ) は
/// `None` = declaration drop ([`parse_border_shorthand`](super::box_model::parse_border_shorthand) と同じ契約)。
///
/// # Initial value fill (省略成分)
///
/// [`BackgroundShorthand`] doc の「Initial value fill」節参照。`visual_boxes`
/// の 0/1/2 occurrence による origin/clip 割り当ては spec §2.10 verbatim
/// ("If one `<visual-box>` value is present then it sets both
/// background-origin and background-clip to that value. If two values are
/// present, then the first sets background-origin and the second
/// background-clip.") — 0 個の場合は 2 longhand それぞれの spec initial
/// (origin: padding-box、clip: border-box) を使う点が 1 個の場合と異なる
/// (1 個の場合は両方その値になるため、0 個の場合だけ非対称)。
pub(crate) fn parse_background_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<BackgroundShorthand> {
    let mut image: Option<BackgroundImage> = None;
    let mut position_and_size: Option<(CssPosition, Option<BackgroundSize>)> = None;
    let mut repeat: Option<BackgroundRepeat> = None;
    let mut attachment: Option<BackgroundAttachment> = None;
    let mut visual_boxes: Vec<VisualBox> = Vec::new();
    let mut color: Option<CssColor> = None;

    loop {
        if image.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<BackgroundImage, ParseError<'_, ()>> {
                parse_background_image(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            image = Some(v);
            continue;
        }
        if position_and_size.is_none()
            && let Ok(v) = input.try_parse(
                |i| -> Result<(CssPosition, Option<BackgroundSize>), ParseError<'_, ()>> {
                    parse_background_position_and_size(i).ok_or_else(|| i.new_custom_error(()))
                },
            )
        {
            position_and_size = Some(v);
            continue;
        }
        if repeat.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<BackgroundRepeat, ParseError<'_, ()>> {
                parse_background_repeat(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            repeat = Some(v);
            continue;
        }
        if attachment.is_none()
            && let Ok(v) =
                input.try_parse(|i| -> Result<BackgroundAttachment, ParseError<'_, ()>> {
                    parse_background_attachment(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            attachment = Some(v);
            continue;
        }
        if visual_boxes.len() < 2
            && let Ok(v) = input.try_parse(|i| -> Result<VisualBox, ParseError<'_, ()>> {
                parse_visual_box(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            visual_boxes.push(v);
            continue;
        }
        if color.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<CssColor, ParseError<'_, ()>> {
                parse_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(v);
            continue;
        }

        // どの unfilled slot にも match しなかった → 埋まっている slot に
        // 対する 2 回目の指定 (`visual_boxes` は 3 回目)、comma (複数 layer)、
        // または未知 token。break で loop 終了、caller の `expect_exhausted`
        // が leftover を drop する (`parse_border_shorthand` と同じ契約)。
        break;
    }

    if image.is_none()
        && position_and_size.is_none()
        && repeat.is_none()
        && attachment.is_none()
        && visual_boxes.is_empty()
        && color.is_none()
    {
        return None;
    }

    let (origin, clip) = match visual_boxes.as_slice() {
        [] => (VisualBox::PaddingBox, VisualBox::BorderBox),
        [one] => (*one, *one),
        [first, second] => (*first, *second),
        // cov:ignore: the loop above only pushes while
        // `visual_boxes.len() < 2`, so `visual_boxes` can never hold more
        // than 2 elements by the time this match runs — there is no input
        // that reaches this arm, and constructing one would require
        // bypassing the loop guard entirely.
        _ => unreachable!("visual_boxes never grows past 2 (loop guard above)"),
    };

    let (position, size) = match position_and_size {
        Some((position, size)) => (
            position,
            size.unwrap_or(BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            }),
        ),
        None => (
            CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
        ),
    };

    Some(BackgroundShorthand {
        color: color.unwrap_or(CssColor::TRANSPARENT),
        image: image.unwrap_or(BackgroundImage::None),
        repeat: repeat.unwrap_or(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        }),
        attachment: attachment.unwrap_or(BackgroundAttachment::Scroll),
        position,
        size,
        clip,
        origin,
    })
}
