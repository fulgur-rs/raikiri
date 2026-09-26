use super::*;

use smol_str::SmolStr;

// ── Skeleton pins ─────────

#[test]
fn target_registry_default_is_empty() {
    // Constructibility check: `TargetRegistry::default()` yields an empty
    // registry. The directive-apply driver will consume this
    // constructor.
    let reg = TargetRegistry::default();
    assert!(reg.resolved.is_empty());
    assert!(reg.pending_slots.is_empty());
    assert_eq!(reg.next_sequence, 0);
    assert_eq!(reg.page_index, 0);
}

#[test]
fn target_registry_shape_matches_design_7_2() {
    // Canonical shape check (design §7.2, lines 1961-1964):
    //   resolved: HashMap<Symbol, TargetInfo>
    //   pending_slots: Vec<TargetSlot>
    // If this test breaks, the type has drifted from the design and
    // that must be reconciled before landing follow-up work.
    // `TargetSlot.id` is the design's `TargetSlotId` (§7.4 line 2098 /
    // a break here means that pairing drifted, not just the two
    // enumerated fields above.
    let mut reg = TargetRegistry::default();
    reg.resolved
        .insert(Symbol::new("fragment-1"), TargetInfo::default());
    reg.pending_slots.push(TargetSlot {
        id: TargetSlotId {
            page_index: 0,
            sequence: 0,
        },
        fragment_id: Symbol::new("fragment-1"),
        request: TargetRequest::Text {
            part: ContentPart::Content,
        },
    });
    assert_eq!(reg.resolved.len(), 1);
    assert_eq!(reg.pending_slots.len(), 1);
    assert!(reg.resolved.contains_key(&Symbol::new("fragment-1")));
}

// ── Fragment parse ──────────────────────────────────────────────

#[test]
fn parse_fragment_accepts_hash_prefixed_ident() {
    assert_eq!(parse_fragment("#chapter-3"), Some("chapter-3"));
    assert_eq!(parse_fragment("#a"), Some("a"));
}

#[test]
fn parse_fragment_rejects_bare_ident_and_external_url() {
    assert_eq!(parse_fragment("chapter-3"), None);
    assert_eq!(parse_fragment("https://example.com/#x"), None);
    assert_eq!(parse_fragment(""), None);
}

#[test]
fn parse_fragment_rejects_empty_fragment() {
    // `#` alone: `strip_prefix` succeeds but leaves `""` — reject so
    // downstream doesn't lookup Symbol::new("") in resolved.
    assert_eq!(parse_fragment("#"), None);
}

// ── Counter formatting ──────────────────────────────────────────

#[test]
fn format_counter_decimal() {
    assert_eq!(format_counter(0, &CounterStyle::Decimal), "0");
    assert_eq!(format_counter(1, &CounterStyle::Decimal), "1");
    assert_eq!(format_counter(42, &CounterStyle::Decimal), "42");
    assert_eq!(format_counter(-3, &CounterStyle::Decimal), "-3");
}

#[test]
fn format_counter_named_unknown_falls_back_to_decimal() {
    // generate-a-counter step 1 <https://www.w3.org/TR/css-counter-styles-3/#generate-a-counter>:
    // "If the counter style is unknown, ... generate a counter
    // representation using the decimal style." Also covers the
    // registry-dependent path (custom @counter-style names — og2
    // comment 2026-07-27 scope (b), unreachable until raikiri-style
    // grows a registry): both an unrecognized keyword and a
    // hypothetical custom name land here identically.
    let style = CounterStyle::Named(SmolStr::new("my-custom-style"));
    assert_eq!(format_counter(4, &style), "4");
}

#[test]
fn format_counter_decimal_leading_zero() {
    let style = CounterStyle::Named(SmolStr::new("decimal-leading-zero"));
    assert_eq!(format_counter(0, &style), "00");
    assert_eq!(format_counter(5, &style), "05");
    assert_eq!(format_counter(15, &style), "15");
    assert_eq!(format_counter(-5, &style), "-05");
    assert_eq!(format_counter(-100, &style), "-100");
}

#[test]
fn format_counter_lower_roman() {
    let style = CounterStyle::Named(SmolStr::new("lower-roman"));
    assert_eq!(format_counter(1, &style), "i");
    assert_eq!(format_counter(4, &style), "iv"); // additive `continue` path (skips `v`)
    assert_eq!(format_counter(1000, &style), "m"); // single-tuple exact `break` path
    assert_eq!(format_counter(3000, &style), "mmm"); // same-weight multi-rep
    assert_eq!(format_counter(3999, &style), "mmmcmxcix"); // full additive-symbols sweep
}

#[test]
fn format_counter_upper_roman() {
    let style = CounterStyle::Named(SmolStr::new("upper-roman"));
    assert_eq!(format_counter(4, &style), "IV");
    assert_eq!(format_counter(3999, &style), "MMMCMXCIX");
}

#[test]
fn format_counter_roman_out_of_range_falls_back_to_decimal() {
    // `range: 1 3999` <https://www.w3.org/TR/css-counter-styles-3/#simple-numeric>:
    // 0, negative, and >3999 are all out of range — fallback renders the
    // *original* signed value as decimal (generate-a-counter step 2
    // uses "the same counter value", not the absolute value).
    let style = CounterStyle::Named(SmolStr::new("lower-roman"));
    assert_eq!(format_counter(0, &style), "0");
    assert_eq!(format_counter(-1, &style), "-1");
    assert_eq!(format_counter(4000, &style), "4000");
}

#[test]
fn format_counter_roman_is_case_insensitive() {
    let style = CounterStyle::Named(SmolStr::new("UPPER-ROMAN"));
    assert_eq!(format_counter(4, &style), "IV");
}

#[test]
fn format_counter_lower_alpha() {
    let style = CounterStyle::Named(SmolStr::new("lower-alpha"));
    assert_eq!(format_counter(1, &style), "a");
    assert_eq!(format_counter(26, &style), "z");
    assert_eq!(format_counter(27, &style), "aa"); // bijective-base-26 wrap
    assert_eq!(format_counter(52, &style), "az");
}

#[test]
fn format_counter_upper_alpha() {
    let style = CounterStyle::Named(SmolStr::new("upper-alpha"));
    assert_eq!(format_counter(1, &style), "A");
    assert_eq!(format_counter(28, &style), "AB");
}

#[test]
fn format_counter_alpha_out_of_range_falls_back_to_decimal() {
    // Alphabetic system range defaults to strictly-positive integers
    // <https://www.w3.org/TR/css-counter-styles-3/#counter-style-range>.
    let style = CounterStyle::Named(SmolStr::new("lower-alpha"));
    assert_eq!(format_counter(0, &style), "0");
    assert_eq!(format_counter(-1, &style), "-1");
}

#[test]
fn format_counter_latin_aliases_match_alpha() {
    // §6.2 defines lower-latin / upper-latin with symbol tables
    // identical to lower-alpha / upper-alpha
    // <https://www.w3.org/TR/css-counter-styles-3/#simple-alphabetic>.
    let lower = CounterStyle::Named(SmolStr::new("lower-latin"));
    let upper = CounterStyle::Named(SmolStr::new("upper-latin"));
    assert_eq!(format_counter(27, &lower), "aa");
    assert_eq!(format_counter(27, &upper), "AA");
}

#[test]
fn format_counter_disc_circle_square_are_value_independent() {
    // `system: cyclic` with a single symbol always renders that symbol,
    // for any value (range -infinity..infinity, no fallback path) —
    // and never the `suffix: " "` from the §6.3 block (see
    // format_named_counter doc: prefix/suffix are a ::marker concern,
    // not part of the counter()/target-counter() string).
    let disc = CounterStyle::Named(SmolStr::new("disc"));
    let circle = CounterStyle::Named(SmolStr::new("circle"));
    let square = CounterStyle::Named(SmolStr::new("square"));
    assert_eq!(format_counter(1, &disc), "\u{2022}");
    assert_eq!(format_counter(-7, &disc), "\u{2022}");
    assert_eq!(format_counter(1, &circle), "\u{25E6}");
    assert_eq!(format_counter(1, &square), "\u{25AA}");
}

/// Test-only shorthand for `CounterStyle::Named(SmolStr::new(name))` —
/// introduced here (sibling divergence noted) because the ce3k
/// additions below construct it ~60 times across 42 new styles; every
/// pre-existing test above keeps the inline form untouched.
fn named(name: &str) -> CounterStyle {
    CounterStyle::Named(SmolStr::new(name))
}

// ── §6.1 Numeric: `numeric`-system digit-table styles ────────────

#[test]
fn format_counter_numeric_digit_styles_value_one() {
    // Cheap transcription-error catcher across all 18 numeric-digit
    // names (17 distinct tables + the `khmer` alias): value 1 must be
    // each table's `digits[1]` entry, per the dfn line's own first
    // `(e.g., ...)` example.
    for (name, first) in [
        ("arabic-indic", '\u{661}'),
        ("bengali", '\u{9e7}'),
        ("cambodian", '\u{17e1}'),
        ("khmer", '\u{17e1}'),
        ("devanagari", '\u{967}'),
        ("gujarati", '\u{ae7}'),
        ("gurmukhi", '\u{a67}'),
        ("kannada", '\u{ce7}'),
        ("lao", '\u{ed1}'),
        ("malayalam", '\u{d67}'),
        ("mongolian", '\u{1811}'),
        ("myanmar", '\u{1041}'),
        ("oriya", '\u{b67}'),
        ("persian", '\u{6f1}'),
        ("tamil", '\u{be7}'),
        ("telugu", '\u{c67}'),
        ("thai", '\u{e51}'),
        ("tibetan", '\u{f21}'),
    ] {
        assert_eq!(
            format_counter(1, &named(name)),
            first.to_string(),
            "style {name} at value 1"
        );
    }
}

#[test]
fn format_counter_numeric_digit_styles_multi_digit_reference_triple() {
    // Each dfn line's `(e.g., ..., 98, 99, 100)` reference triple,
    // spot-checked on 3 of the 18 tables (arabic-indic, bengali,
    // tibetan) — exercises the positional two/three-digit path, not
    // just the single-digit case above.
    assert_eq!(
        format_counter(98, &named("arabic-indic")),
        "\u{669}\u{668}" // ٩٨
    );
    assert_eq!(
        format_counter(99, &named("arabic-indic")),
        "\u{669}\u{669}" // ٩٩
    );
    assert_eq!(
        format_counter(100, &named("arabic-indic")),
        "\u{661}\u{660}\u{660}" // ١٠٠
    );
    assert_eq!(format_counter(98, &named("bengali")), "\u{9ef}\u{9ee}"); // ৯৮
    assert_eq!(
        format_counter(100, &named("bengali")),
        "\u{9e7}\u{9e6}\u{9e6}" // ১০০
    );
    assert_eq!(
        format_counter(100, &named("tibetan")),
        "\u{f21}\u{f20}\u{f20}" // ༡༠༠
    );
}

#[test]
fn format_counter_numeric_digit_styles_negative_uses_ascii_hyphen() {
    // `numeric` system default `negative: "-"` — none of these
    // @counter-style blocks override it.
    assert_eq!(format_counter(-1, &named("arabic-indic")), "-\u{661}");
    assert_eq!(format_counter(0, &named("bengali")), "\u{9e6}");
}

#[test]
fn format_counter_cjk_decimal_is_positional_not_longhand() {
    // §6.1 dfn: "(e.g., 一, 二, 三, ..., 九八, 九九, 一〇〇)" — digit
    // substitution, not the longhand-additive algorithm §7.1 uses for
    // otherwise-similar Han-numeral styles.
    assert_eq!(format_counter(1, &named("cjk-decimal")), "\u{4e00}"); // 一
    assert_eq!(format_counter(2, &named("cjk-decimal")), "\u{4e8c}"); // 二
    assert_eq!(
        format_counter(98, &named("cjk-decimal")),
        "\u{4e5d}\u{516b}" // 九八
    );
    assert_eq!(
        format_counter(100, &named("cjk-decimal")),
        "\u{4e00}\u{3007}\u{3007}" // 一〇〇
    );
}

#[test]
fn format_counter_cjk_decimal_negative_falls_back_to_decimal() {
    // `range: 0 infinite` override — negative is out of range, and the
    // unset `fallback` descriptor defaults to `decimal`.
    assert_eq!(format_counter(-1, &named("cjk-decimal")), "-1");
}

// ── §6.1 Numeric: `additive`-system styles ────────────────────────

#[test]
fn format_counter_armenian_reference_triple_and_alias() {
    // dfn: "(e.g., Ա, Բ, Գ, ..., ՂԸ, ՂԹ, Ճ)" for both `armenian` and
    // `upper-armenian` (`system: extends armenian` — same table).
    for name in ["armenian", "upper-armenian"] {
        assert_eq!(format_counter(1, &named(name)), "\u{531}"); // Ա
        assert_eq!(format_counter(98, &named(name)), "\u{542}\u{538}"); // ՂԸ (90+8)
        assert_eq!(format_counter(99, &named(name)), "\u{542}\u{539}"); // ՂԹ (90+9)
        assert_eq!(format_counter(100, &named(name)), "\u{543}"); // Ճ
    }
}

#[test]
fn format_counter_lower_armenian_reference_triple() {
    // dfn: "(e.g., ա, բ, գ, ..., ղը, ղթ, ճ)".
    assert_eq!(format_counter(1, &named("lower-armenian")), "\u{561}"); // ա
    assert_eq!(
        format_counter(98, &named("lower-armenian")),
        "\u{572}\u{568}" // ղը
    );
    assert_eq!(format_counter(100, &named("lower-armenian")), "\u{573}"); // ճ
}

#[test]
fn format_counter_armenian_range_boundaries() {
    // `range: 1 9999` shared by armenian/upper-armenian/lower-armenian.
    // 9999 = 9000 + 900 + 90 + 9 (one rep of each weight — the greedy
    // additive algorithm never repeats a weight here since Armenian's
    // table has an exact tuple for every non-zero decimal digit at
    // every place value).
    assert_eq!(
        format_counter(9999, &named("armenian")),
        "\u{554}\u{54b}\u{542}\u{539}" // ՔՋՂԹ
    );
    assert_eq!(format_counter(0, &named("armenian")), "0");
    assert_eq!(format_counter(10000, &named("armenian")), "10000");
}

#[test]
fn format_counter_georgian_reference_triple_and_irregular_weight_8() {
    // dfn: "(e.g., ა, ბ, გ, ..., ჟჱ, ჟთ, რ)" — 98 uses weight-8 = ჱ
    // (U+10F1, a dedicated numeral letter reserved for 8), NOT the
    // "natural" 9th-place-in-alphabet character.
    assert_eq!(format_counter(1, &named("georgian")), "\u{10d0}"); // ა
    assert_eq!(
        format_counter(98, &named("georgian")),
        "\u{10df}\u{10f1}" // ჟჱ (90 + 8)
    );
    assert_eq!(
        format_counter(99, &named("georgian")),
        "\u{10df}\u{10d7}" // ჟთ (90 + 9)
    );
    assert_eq!(format_counter(100, &named("georgian")), "\u{10e0}"); // რ
    // 19999 = 10000 + 9000 + 900 + 90 + 9, one rep of each weight.
    assert_eq!(
        format_counter(19999, &named("georgian")),
        "\u{10f5}\u{10f0}\u{10e8}\u{10df}\u{10d7}" // ჵჰშჟთ
    );
    assert_eq!(format_counter(20000, &named("georgian")), "20000"); // range: 1 19999
}

#[test]
fn format_counter_hebrew_reference_triple() {
    // dfn: "(e.g., א‎, ב‎, ג‎, ..., צח‎, צט‎, ק‎)".
    assert_eq!(format_counter(1, &named("hebrew")), "\u{5d0}"); // א
    assert_eq!(
        format_counter(98, &named("hebrew")),
        "\u{5e6}\u{5d7}" // צח (90 + 8)
    );
    assert_eq!(
        format_counter(99, &named("hebrew")),
        "\u{5e6}\u{5d8}" // צט (90 + 9)
    );
    assert_eq!(format_counter(100, &named("hebrew")), "\u{5e7}"); // ק
}

#[test]
fn format_counter_hebrew_manual_override_avoids_tetragrammaton() {
    // 15/16 are manually specified as 9+6 / 9+7 rather than the
    // "natural" 10+5 / 10+6 additive decomposition, specifically to
    // avoid the two-letter combination that resembles the
    // Tetragrammaton (spec's own note on the @counter-style block).
    assert_eq!(
        format_counter(15, &named("hebrew")),
        "\u{5d8}\u{5d5}" // טו (9 + 6), not "\u{5d9}\u{5d4}" (10 + 5)
    );
    assert_eq!(
        format_counter(16, &named("hebrew")),
        "\u{5d8}\u{5d6}" // טז (9 + 7), not "\u{5d9}\u{5d5}" (10 + 6)
    );
    assert_eq!(format_counter(17, &named("hebrew")), "\u{5d9}\u{5d6}"); // יז
    assert_eq!(format_counter(19, &named("hebrew")), "\u{5d9}\u{5d8}"); // יט
}

#[test]
fn format_counter_hebrew_range_boundary_falls_back_to_decimal() {
    // 10999 = 10000 + 400 + 400 + 100 + 90 + 9 (greedy: the 400-weight
    // tuple is used twice — floor(999/400) = 2, remaining 199 — then
    // 100 + 90 + 9 exactly).
    assert_eq!(
        format_counter(10999, &named("hebrew")),
        "\u{5d9}\u{5f3}\u{5ea}\u{5ea}\u{5e7}\u{5e6}\u{5d8}" // י׳תתקצט
    );
    assert_eq!(format_counter(11000, &named("hebrew")), "11000"); // range: 1 10999
    assert_eq!(format_counter(0, &named("hebrew")), "0");
}

#[test]
fn format_counter_additive_named_style_is_case_insensitive() {
    assert_eq!(format_counter(1, &named("ARMENIAN")), "\u{531}");
}

// ── §6.2 Alphabetic: `alphabetic`-system styles beyond alpha/latin ─

#[test]
fn format_counter_lower_greek_wraps_at_24() {
    // dfn: "(e.g., α, β, γ, ..., ω, αα, αβ)" — 24 symbols (final sigma
    // ς deliberately absent), so ω is the 24th, not the usual
    // Greek-alphabet-adjacent 25th.
    assert_eq!(format_counter(1, &named("lower-greek")), "\u{3b1}"); // α
    assert_eq!(format_counter(24, &named("lower-greek")), "\u{3c9}"); // ω
    assert_eq!(format_counter(25, &named("lower-greek")), "\u{3b1}\u{3b1}"); // αα
    assert_eq!(format_counter(26, &named("lower-greek")), "\u{3b1}\u{3b2}"); // αβ
}

#[test]
fn format_counter_hiragana_wraps_at_48() {
    // dfn: "(e.g., あ, い, う, ..., ん, ああ, あい)".
    assert_eq!(format_counter(1, &named("hiragana")), "\u{3042}"); // あ
    assert_eq!(format_counter(48, &named("hiragana")), "\u{3093}"); // ん
    assert_eq!(format_counter(49, &named("hiragana")), "\u{3042}\u{3042}"); // ああ
    assert_eq!(format_counter(50, &named("hiragana")), "\u{3042}\u{3044}"); // あい
}

#[test]
fn format_counter_hiragana_iroha_wraps_at_47() {
    // dfn: "(e.g., い, ろ, は, ..., す, いい, いろ)" — 47 symbols (ん
    // excluded from iroha order), so す (not ん) is the 47th.
    assert_eq!(format_counter(1, &named("hiragana-iroha")), "\u{3044}"); // い
    assert_eq!(format_counter(47, &named("hiragana-iroha")), "\u{3059}"); // す
    assert_eq!(
        format_counter(48, &named("hiragana-iroha")),
        "\u{3044}\u{3044}" // いい
    );
    assert_eq!(
        format_counter(49, &named("hiragana-iroha")),
        "\u{3044}\u{308d}" // いろ
    );
}

#[test]
fn format_counter_katakana_wraps_at_48() {
    // dfn: "(e.g., ア, イ, ウ, ..., ン, アア, アイ)".
    assert_eq!(format_counter(1, &named("katakana")), "\u{30a2}"); // ア
    assert_eq!(format_counter(48, &named("katakana")), "\u{30f3}"); // ン
    assert_eq!(format_counter(49, &named("katakana")), "\u{30a2}\u{30a2}"); // アア
}

#[test]
fn format_counter_katakana_iroha_wraps_at_47() {
    // dfn: "(e.g., イ, ロ, ハ, ..., ス, イイ, イロ)".
    assert_eq!(format_counter(1, &named("katakana-iroha")), "\u{30a4}"); // イ
    assert_eq!(format_counter(47, &named("katakana-iroha")), "\u{30b9}"); // ス
    assert_eq!(
        format_counter(48, &named("katakana-iroha")),
        "\u{30a4}\u{30a4}" // イイ
    );
}

#[test]
fn format_counter_alphabetic_extended_out_of_range_falls_back_to_decimal() {
    // Same strictly-positive-integer range as lower-alpha/upper-alpha.
    assert_eq!(format_counter(0, &named("lower-greek")), "0");
    assert_eq!(format_counter(-1, &named("hiragana")), "-1");
}

// ── §6.4 Fixed: `fixed`-system styles ─────────────────────────────

#[test]
fn format_counter_cjk_earthly_branch_full_range() {
    // dfn: "(e.g., 子, 丑, 寅, ..., 亥)" — 12 symbols, first value 1.
    assert_eq!(format_counter(1, &named("cjk-earthly-branch")), "\u{5b50}"); // 子
    assert_eq!(format_counter(12, &named("cjk-earthly-branch")), "\u{4ea5}"); // 亥
}

#[test]
fn format_counter_cjk_heavenly_stem_full_range() {
    // dfn: "(e.g., 甲, 乙, 丙, ..., 癸)" — 10 symbols, first value 1.
    assert_eq!(format_counter(1, &named("cjk-heavenly-stem")), "\u{7532}"); // 甲
    assert_eq!(format_counter(10, &named("cjk-heavenly-stem")), "\u{7678}"); // 癸
}

#[test]
fn format_counter_fixed_styles_exhausted_range_falls_back_to_decimal() {
    // Once the 12/10-symbol list is exhausted, `fixed` "cannot
    // represent" further values — decimal fallback, same as 0/negative.
    assert_eq!(format_counter(13, &named("cjk-earthly-branch")), "13");
    assert_eq!(format_counter(11, &named("cjk-heavenly-stem")), "11");
    assert_eq!(format_counter(0, &named("cjk-earthly-branch")), "0");
    assert_eq!(format_counter(-1, &named("cjk-heavenly-stem")), "-1");
}

// ── §7.1 Longhand East Asian: Japanese/Korean `additive` styles ───

#[test]
fn format_counter_japanese_informal_ten_column_reference() {
    // §7.1's 10-column reference table (0,1,2,3,10,11,99,100,101,6001).
    let style = named("japanese-informal");
    assert_eq!(format_counter(0, &style), "\u{3007}"); // 〇
    assert_eq!(format_counter(1, &style), "\u{4e00}"); // 一
    assert_eq!(format_counter(10, &style), "\u{5341}"); // 十
    assert_eq!(format_counter(11, &style), "\u{5341}\u{4e00}"); // 十一
    assert_eq!(format_counter(99, &style), "\u{4e5d}\u{5341}\u{4e5d}"); // 九十九
    assert_eq!(format_counter(100, &style), "\u{767e}"); // 百 (no leading 一, additive table's own symbol)
    assert_eq!(format_counter(101, &style), "\u{767e}\u{4e00}"); // 百一
    assert_eq!(format_counter(6001, &style), "\u{516d}\u{5343}\u{4e00}"); // 六千一 — NO zero-digit marker
}

#[test]
fn format_counter_japanese_formal_ten_column_reference() {
    let style = named("japanese-formal");
    assert_eq!(format_counter(0, &style), "\u{96f6}"); // 零
    assert_eq!(format_counter(1, &style), "\u{58f1}"); // 壱
    assert_eq!(format_counter(11, &style), "\u{58f1}\u{62fe}\u{58f1}"); // 壱拾壱
    assert_eq!(format_counter(100, &style), "\u{58f1}\u{767e}"); // 壱百
    assert_eq!(
        format_counter(6001, &style),
        "\u{516d}\u{9621}\u{58f1}" // 六阡壱
    );
}

#[test]
fn format_counter_korean_hangul_formal_ten_column_reference() {
    let style = named("korean-hangul-formal");
    assert_eq!(format_counter(0, &style), "\u{c601}"); // 영
    assert_eq!(format_counter(1, &style), "\u{c77c}"); // 일
    assert_eq!(
        format_counter(11, &style),
        "\u{c77c}\u{c2ed}\u{c77c}" // 일십일
    );
    assert_eq!(format_counter(100, &style), "\u{c77c}\u{bc31}"); // 일백
    assert_eq!(
        format_counter(6001, &style),
        "\u{c721}\u{cc9c}\u{c77c}" // 육천일
    );
}

#[test]
fn format_counter_korean_hanja_informal_and_formal_ten_column_reference() {
    let informal = named("korean-hanja-informal");
    let formal = named("korean-hanja-formal");
    assert_eq!(format_counter(11, &informal), "\u{5341}\u{4e00}"); // 十一
    assert_eq!(format_counter(6001, &informal), "\u{516d}\u{5343}\u{4e00}"); // 六千一
    assert_eq!(
        format_counter(11, &formal),
        "\u{58f9}\u{62fe}\u{58f9}" // 壹拾壹
    );
    assert_eq!(
        format_counter(6001, &formal),
        "\u{516d}\u{4edf}\u{58f9}" // 六仟壹
    );
}

#[test]
fn format_counter_longhand_additive_negative_uses_style_specific_prefix() {
    assert_eq!(
        format_counter(-1, &named("japanese-informal")),
        "\u{30de}\u{30a4}\u{30ca}\u{30b9}\u{4e00}" // マイナス一
    );
    assert_eq!(
        format_counter(-11, &named("korean-hangul-formal")),
        "\u{b9c8}\u{c774}\u{b108}\u{c2a4}  \u{c77c}\u{c2ed}\u{c77c}" // 마이너스  일십일 (two spaces)
    );
}

#[test]
fn format_counter_longhand_additive_out_of_range_falls_back_through_cjk_decimal() {
    // Two-hop chain: out of `-9999..=9999` → cjk-decimal (positional);
    // cjk-decimal's own `range: 0 infinite` still covers this positive
    // out-of-range value, so it does NOT fall through to plain decimal
    // here — 10000 renders as cjk-decimal's "一〇〇〇〇", not "10000".
    assert_eq!(
        format_counter(10000, &named("japanese-informal")),
        "\u{4e00}\u{3007}\u{3007}\u{3007}\u{3007}" // 一〇〇〇〇 (cjk-decimal)
    );
    // Negative out of range: cjk-decimal also rejects it (range starts
    // at 0), so the chain falls all the way through to plain decimal.
    assert_eq!(
        format_counter(-10000, &named("japanese-informal")),
        "-10000"
    );
}

// ── §7.1.3 Longhand East Asian: Chinese digit-marker styles ───────

#[test]
fn format_counter_simp_chinese_informal_ten_column_reference() {
    let style = named("simp-chinese-informal");
    assert_eq!(format_counter(0, &style), "\u{96f6}"); // 零
    assert_eq!(format_counter(1, &style), "\u{4e00}"); // 一
    assert_eq!(format_counter(10, &style), "\u{5341}"); // 十
    assert_eq!(format_counter(11, &style), "\u{5341}\u{4e00}"); // 十一
    assert_eq!(format_counter(99, &style), "\u{4e5d}\u{5341}\u{4e5d}"); // 九十九
    assert_eq!(format_counter(100, &style), "\u{4e00}\u{767e}"); // 一百 (leading 一, unlike japanese-informal)
    assert_eq!(
        format_counter(101, &style),
        "\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 一百零一
    );
    assert_eq!(
        format_counter(6001, &style),
        "\u{516d}\u{5343}\u{96f6}\u{4e00}" // 六千零一 — HAS the zero-digit marker
    );
}

#[test]
fn format_counter_simp_chinese_informal_first_120_values_spot_check() {
    // Spot-checks against the spec's own first-120-values worked
    // table, hitting the 10-19 tens-digit-removal boundary and the
    // point where 110/111 do NOT get that same removal.
    let style = named("simp-chinese-informal");
    assert_eq!(format_counter(9, &style), "\u{4e5d}"); // 九
    assert_eq!(format_counter(10, &style), "\u{5341}"); // 十
    assert_eq!(format_counter(19, &style), "\u{5341}\u{4e5d}"); // 十九
    assert_eq!(format_counter(20, &style), "\u{4e8c}\u{5341}"); // 二十
    assert_eq!(format_counter(21, &style), "\u{4e8c}\u{5341}\u{4e00}"); // 二十一
    assert_eq!(
        format_counter(109, &style),
        "\u{4e00}\u{767e}\u{96f6}\u{4e5d}"
    ); // 一百零九
    assert_eq!(
        format_counter(110, &style),
        "\u{4e00}\u{767e}\u{4e00}\u{5341}"
    ); // 一百一十 (no removal at 110)
    assert_eq!(
        format_counter(111, &style),
        "\u{4e00}\u{767e}\u{4e00}\u{5341}\u{4e00}" // 一百一十一
    );
    assert_eq!(
        format_counter(120, &style),
        "\u{4e00}\u{767e}\u{4e8c}\u{5341}"
    ); // 一百二十
}

#[test]
fn format_counter_simp_chinese_formal_does_not_drop_tens_digit() {
    // Formal styles skip algorithm step 3 (informal-only tens-digit
    // removal) entirely — 10/11 keep their leading 壹.
    let style = named("simp-chinese-formal");
    assert_eq!(format_counter(10, &style), "\u{58f9}\u{62fe}"); // 壹拾
    assert_eq!(format_counter(11, &style), "\u{58f9}\u{62fe}\u{58f9}"); // 壹拾壹
    assert_eq!(format_counter(100, &style), "\u{58f9}\u{4f70}"); // 壹佰
    assert_eq!(
        format_counter(101, &style),
        "\u{58f9}\u{4f70}\u{96f6}\u{58f9}" // 壹佰零壹
    );
    assert_eq!(
        format_counter(6001, &style),
        "\u{9646}\u{4edf}\u{96f6}\u{58f9}" // 陆仟零壹 (simp 6 = 陆)
    );
}

#[test]
fn format_counter_trad_chinese_formal_uses_traditional_glyphs() {
    // Differs from simp-chinese-formal only in digits 2/3/6 and the
    // negative sign — verified via digit 6 (simp 陆 U+9646 vs trad 陸
    // U+9678) and the negative sign (简负 U+8D1F vs 繁負 U+8CA0).
    let style = named("trad-chinese-formal");
    assert_eq!(
        format_counter(6001, &style),
        "\u{9678}\u{4edf}\u{96f6}\u{58f9}" // 陸仟零壹
    );
    assert_eq!(
        format_counter(-1, &style),
        "\u{8ca0}\u{58f9}" // 負壹
    );
}

#[test]
fn format_counter_trad_chinese_informal_matches_simp_except_negative_sign() {
    // `trad-chinese-informal`'s digit/marker table is byte-identical to
    // `simp-chinese-informal`'s (informal-style basic numerals don't
    // differ between simplified/traditional) — the only distinguishing
    // field is the negative sign (負 U+8CA0 vs 负 U+8D1F). A
    // positive-value assertion alone couldn't tell this entry apart
    // from an accidental `simp-chinese-informal` reuse, so the
    // negative case is load-bearing here.
    let style = named("trad-chinese-informal");
    assert_eq!(
        format_counter(6001, &style),
        "\u{516d}\u{5343}\u{96f6}\u{4e00}" // 六千零一
    );
    assert_eq!(
        format_counter(-101, &style),
        "\u{8ca0}\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 負一百零一
    );
}

#[test]
fn format_counter_cjk_ideographic_matches_trad_chinese_informal() {
    // §7.1.3's own dfn: "cjk-ideographic ... identical to
    // trad-chinese-informal. (It exists for legacy reasons.)" — check
    // both a positive and the negative case (same rationale as the
    // trad-chinese-informal-vs-simp-chinese-informal test above: a
    // positive-only assertion can't distinguish "correctly aliased to
    // trad-chinese-informal" from "accidentally aliased to
    // simp-chinese-informal", since the two tables only diverge on the
    // negative-sign glyph).
    let style = named("cjk-ideographic");
    assert_eq!(
        format_counter(6001, &style),
        "\u{516d}\u{5343}\u{96f6}\u{4e00}" // 六千零一
    );
    assert_eq!(
        format_counter(-101, &style),
        "\u{8ca0}\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 負一百零一
    );
}

#[test]
fn format_counter_chinese_digit_marker_negative_uses_single_char_prefix() {
    assert_eq!(
        format_counter(-101, &named("simp-chinese-informal")),
        "\u{8d1f}\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 负一百零一
    );
}

#[test]
fn format_counter_chinese_digit_marker_out_of_range_falls_back_through_cjk_decimal() {
    assert_eq!(
        format_counter(10000, &named("simp-chinese-informal")),
        "\u{4e00}\u{3007}\u{3007}\u{3007}\u{3007}" // cjk-decimal 一〇〇〇〇
    );
    assert_eq!(
        format_counter(-10000, &named("trad-chinese-formal")),
        "-10000" // cjk-decimal also rejects negative, falls to plain decimal
    );
}

// ── §7.2 Ethiopic Numeric Counter Style ───────────────────────────

#[test]
fn format_counter_ethiopic_numeric_value_one_special_case() {
    assert_eq!(format_counter(1, &named("ethiopic-numeric")), "\u{1369}"); // ፩
}

#[test]
fn format_counter_ethiopic_numeric_spec_worked_examples() {
    // The spec's own first two worked examples fit `i32` directly.
    assert_eq!(format_counter(100, &named("ethiopic-numeric")), "\u{137b}"); // ፻
    assert_eq!(
        format_counter(78_010_092, &named("ethiopic-numeric")),
        "\u{1378}\u{1370}\u{137b}\u{1369}\u{137c}\u{137a}\u{136a}" // ፸፰፻፩፼፺፪
    );
    // The spec's third example (780100000092 → ...፼፼...) doesn't fit
    // `i32` (counter values are `i32` throughout this file), so it
    // can't be exercised through this API. 10000092 substitutes: same
    // asymmetry (an even, non-zero index group with value 0 still
    // emits a bare ፼ with no preceding digit — group 2 here), verified
    // in-range by hand-tracing the algorithm: groups (LSB-first)
    // `[92, 0, 0, 10]`; group 3 (msb, odd, value 10) → ፲፻; group 2
    // (even, index != 0, value 0) → suppressed digit, but it still
    // gets a bare ፼; group 1 (odd, value 0) → fully suppressed
    // (exception applies to ፻ only); group 0 → ፺፪.
    assert_eq!(
        format_counter(10_000_092, &named("ethiopic-numeric")),
        "\u{1372}\u{137b}\u{137c}\u{137a}\u{136a}" // ፲፻፼፺፪
    );
}

#[test]
fn format_counter_ethiopic_numeric_out_of_range_falls_back_to_decimal() {
    // `range: 1 infinite` — 0 and negative are out of range.
    assert_eq!(format_counter(0, &named("ethiopic-numeric")), "0");
    assert_eq!(format_counter(-1, &named("ethiopic-numeric")), "-1");
}

#[test]
fn join_counter_stack_decimal_with_dot_separator() {
    assert_eq!(
        join_counter_stack(&[1, 2, 3], ".", &CounterStyle::Decimal),
        "1.2.3"
    );
    assert_eq!(join_counter_stack(&[5], ".", &CounterStyle::Decimal), "5");
    assert_eq!(join_counter_stack(&[], ".", &CounterStyle::Decimal), "");
}

#[test]
fn join_counter_stack_non_decimal_style() {
    // target-counters() path (TargetRequest::Counters) threads `style`
    // through join_counter_stack the same as the single-value path —
    // check that a non-decimal style formats every level, not just the
    // leaf.
    let style = CounterStyle::Named(SmolStr::new("lower-alpha"));
    assert_eq!(join_counter_stack(&[1, 2, 3], ".", &style), "a.b.c");
}

// ── target-counter resolve (immediate path) ─────────────────────

fn make_info(counters: &[(&str, &[i32])], texts: &[(ContentPart, &str)]) -> TargetInfo {
    let mut info = TargetInfo::default();
    for (name, stack) in counters {
        info.counters.insert(Symbol::new(*name), stack.to_vec());
    }
    for (part, text) in texts {
        info.text_parts.push((*part, (*text).to_owned()));
    }
    info
}

#[test]
fn resolve_target_counter_returns_leaf_of_stack() {
    let mut reg = TargetRegistry::default();
    reg.register(
        Symbol::new("chapter-3"),
        make_info(&[("chapter", &[1, 2, 3])], &[]),
    );
    let out =
        reg.resolve_target_counter("#chapter-3", Symbol::new("chapter"), CounterStyle::Decimal);
    assert_eq!(out, ResolveOutcome::Resolved("3".to_owned()));
    // Immediate resolve must not queue a pending slot.
    assert!(reg.pending_slots.is_empty());
}

#[test]
fn resolve_target_counter_missing_counter_defaults_to_zero() {
    let mut reg = TargetRegistry::default();
    reg.register(Symbol::new("chapter-3"), make_info(&[], &[]));
    let out =
        reg.resolve_target_counter("#chapter-3", Symbol::new("chapter"), CounterStyle::Decimal);
    assert_eq!(out, ResolveOutcome::Resolved("0".to_owned()));
}

#[test]
fn resolve_target_counter_non_fragment_url_falls_back_to_empty() {
    let mut reg = TargetRegistry::default();
    let out = reg.resolve_target_counter(
        "https://example.com/",
        Symbol::new("chapter"),
        CounterStyle::Decimal,
    );
    assert_eq!(out, ResolveOutcome::Resolved(String::new()));
    // Fallback must not queue a pending slot (would leak into flush).
    assert!(reg.pending_slots.is_empty());
}

// ── target-counters resolve (immediate path) ────────────────────

#[test]
fn resolve_target_counters_joins_full_stack() {
    let mut reg = TargetRegistry::default();
    reg.register(
        Symbol::new("sec-1-2-3"),
        make_info(&[("section", &[1, 2, 3])], &[]),
    );
    let out = reg.resolve_target_counters(
        "#sec-1-2-3",
        Symbol::new("section"),
        ".",
        CounterStyle::Decimal,
    );
    assert_eq!(out, ResolveOutcome::Resolved("1.2.3".to_owned()));
}

#[test]
fn resolve_target_counters_absent_counter_yields_zero() {
    // CSS Content 3 §2.6.2: an undefined counter has value 0;
    // `counters(name, sep)` renders that as the single formatted "0"
    // (join-with-sep of a one-element `[0]`) — NOT the empty string.
    // Regression check against a prior implementation that used
    // `.unwrap_or_default()` on the joined output.
    let mut reg = TargetRegistry::default();
    reg.register(Symbol::new("sec-1"), make_info(&[], &[]));
    let out =
        reg.resolve_target_counters("#sec-1", Symbol::new("section"), ".", CounterStyle::Decimal);
    assert_eq!(out, ResolveOutcome::Resolved("0".to_owned()));
}

#[test]
fn resolve_target_counters_empty_stack_yields_zero() {
    // Distinct code path from "absent counter": the counter *is* present
    // in `counters` but its stack is an empty Vec. Same CSS §2.6.2 rule
    // applies — render as "0".
    let mut reg = TargetRegistry::default();
    reg.register(Symbol::new("sec-1"), make_info(&[("section", &[])], &[]));
    let out =
        reg.resolve_target_counters("#sec-1", Symbol::new("section"), ".", CounterStyle::Decimal);
    assert_eq!(out, ResolveOutcome::Resolved("0".to_owned()));
}

#[test]
fn resolve_target_counters_non_fragment_url_falls_back_to_empty() {
    // Analogous to resolve_target_counter_non_fragment_url_falls_back_to_empty:
    // non-fragment URLs (external, malformed) are unresolvable
    // per-document and must NOT queue a pending slot (would grow
    // pending_slots unbounded).
    let mut reg = TargetRegistry::default();
    let out = reg.resolve_target_counters(
        "https://example.com/",
        Symbol::new("section"),
        ".",
        CounterStyle::Decimal,
    );
    assert_eq!(out, ResolveOutcome::Resolved(String::new()));
    assert!(reg.pending_slots.is_empty());
}

// ── target-text resolve (immediate path) ────────────────────────

#[test]
fn resolve_target_text_returns_content_part() {
    let mut reg = TargetRegistry::default();
    reg.register(
        Symbol::new("h1"),
        make_info(
            &[],
            &[
                (ContentPart::Content, "Introduction"),
                (ContentPart::Before, "§"),
            ],
        ),
    );
    assert_eq!(
        reg.resolve_target_text("#h1", ContentPart::Content),
        ResolveOutcome::Resolved("Introduction".to_owned())
    );
    assert_eq!(
        reg.resolve_target_text("#h1", ContentPart::Before),
        ResolveOutcome::Resolved("§".to_owned())
    );
}

#[test]
fn resolve_target_text_absent_part_yields_empty_string() {
    let mut reg = TargetRegistry::default();
    reg.register(Symbol::new("h1"), make_info(&[], &[]));
    assert_eq!(
        reg.resolve_target_text("#h1", ContentPart::Content),
        ResolveOutcome::Resolved(String::new())
    );
}

#[test]
fn resolve_target_text_non_fragment_url_falls_back_to_empty() {
    // Analogous to the counter/counters non-fragment tests: unresolvable
    // external URLs must return an empty string synchronously without
    // queueing a pending slot.
    let mut reg = TargetRegistry::default();
    let out = reg.resolve_target_text("https://example.com/", ContentPart::Content);
    assert_eq!(out, ResolveOutcome::Resolved(String::new()));
    assert!(reg.pending_slots.is_empty());
}

// ── Pending / flush_pending (forward-reference path) ────────────

#[test]
fn forward_reference_yields_pending_then_flushes_to_resolved() {
    let mut reg = TargetRegistry::default();

    // Site A resolves *before* the register site has been walked.
    let out_a =
        reg.resolve_target_counter("#chapter-3", Symbol::new("chapter"), CounterStyle::Decimal);
    let id_a = match out_a {
        ResolveOutcome::Pending(s) => s,
        ResolveOutcome::Resolved(_) => panic!("expected Pending, got Resolved"),
    };
    assert_eq!(
        id_a,
        TargetSlotId {
            page_index: 0,
            sequence: 0,
        }
    );
    assert_eq!(reg.pending_slots.len(), 1);

    // Second forward reference to a different fragment / kind.
    let out_b = reg.resolve_target_text("#chapter-3", ContentPart::Content);
    let id_b = match out_b {
        ResolveOutcome::Pending(s) => s,
        ResolveOutcome::Resolved(_) => panic!("expected Pending, got Resolved"),
    };
    assert_eq!(
        id_b,
        TargetSlotId {
            page_index: 0,
            sequence: 1,
        }
    );
    assert_eq!(reg.pending_slots.len(), 2);

    // Walk lands the register.
    reg.register(
        Symbol::new("chapter-3"),
        make_info(
            &[("chapter", &[1, 2, 3])],
            &[(ContentPart::Content, "The Middle")],
        ),
    );

    // Flush drains both pending slots and returns their sequenced values.
    let resolutions = reg.flush_pending();
    assert_eq!(resolutions.len(), 2);
    assert!(reg.pending_slots.is_empty());
    assert_eq!(
        resolutions[0],
        PendingResolution {
            slot_id: id_a,
            value: Some("3".to_owned()),
        }
    );
    assert_eq!(
        resolutions[1],
        PendingResolution {
            slot_id: id_b,
            value: Some("The Middle".to_owned()),
        }
    );
}

#[test]
fn flush_pending_retains_unregistered_slots() {
    // Streaming intent (design §7.4): a slot whose fragment never landed
    // must NOT be dropped — it may resolve in a later register + flush
    // cycle (e.g. next batch, next Consumer iteration).
    let mut reg = TargetRegistry::default();
    let out = reg.resolve_target_counter("#never", Symbol::new("chapter"), CounterStyle::Decimal);
    assert!(matches!(out, ResolveOutcome::Pending(_)));
    assert_eq!(reg.pending_slots.len(), 1);

    // First flush: no register, slot retained.
    let resolutions = reg.flush_pending();
    assert!(resolutions.is_empty());
    assert_eq!(reg.pending_slots.len(), 1);

    // Later register lands the fragment; second flush resolves it.
    reg.register(Symbol::new("never"), make_info(&[("chapter", &[7])], &[]));
    let resolutions = reg.flush_pending();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].value.as_deref(), Some("7"));
    assert!(reg.pending_slots.is_empty());
}

#[test]
fn register_first_wins_on_duplicate_fragment_id() {
    // DOM id resolution is first-in-tree-order (HTML §3.2.6.1 —
    // duplicate ids are invalid HTML but `getElementById` still resolves
    // to the first element in tree order). register() is expected to be
    // called in tree order by the register-site walker; last-wins would
    // cause target-* to resolve against a later duplicate.
    let mut reg = TargetRegistry::default();
    reg.register(Symbol::new("dup"), make_info(&[("chapter", &[1])], &[]));
    // Second register call for the same fragment must be a no-op.
    reg.register(
        Symbol::new("dup"),
        make_info(&[("chapter", &[99])], &[(ContentPart::Content, "later")]),
    );

    // Counter resolves against the first (winning) register.
    let out = reg.resolve_target_counter("#dup", Symbol::new("chapter"), CounterStyle::Decimal);
    assert_eq!(out, ResolveOutcome::Resolved("1".to_owned()));

    // Text side: the first info had no ContentPart::Content, so the
    // fallback is "" — NOT "later" from the second (losing) info.
    // This pins that or_insert isn't quietly merging fields.
    let out_text = reg.resolve_target_text("#dup", ContentPart::Content);
    assert_eq!(out_text, ResolveOutcome::Resolved(String::new()));
}

#[test]
fn flush_pending_partial_preserves_order_and_retains_unresolved() {
    // Mixed batch: some slots resolve, some don't. The flush must
    //   (a) emit resolutions in slot-queued order (sequence 0 before 1
    //       before 3, etc.),
    //   (b) retain unresolved slots in `pending_slots` for a later
    //       flush cycle,
    //   (c) not disturb the retained slots' original ordering.
    let mut reg = TargetRegistry::default();

    // Queue three slots for different fragments in this order:
    //   seq 0: #a  counter
    //   seq 1: #b  counters
    //   seq 2: #c  text
    //   seq 3: #a  text  (same fragment as seq 0)
    let id_a = match reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal)
    {
        ResolveOutcome::Pending(s) => s,
        ResolveOutcome::Resolved(_) => panic!("expected Pending"),
    };
    let id_b =
        match reg.resolve_target_counters("#b", Symbol::new("section"), "-", CounterStyle::Decimal)
        {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
    let id_c = match reg.resolve_target_text("#c", ContentPart::Content) {
        ResolveOutcome::Pending(s) => s,
        ResolveOutcome::Resolved(_) => panic!("expected Pending"),
    };
    let id_a2 = match reg.resolve_target_text("#a", ContentPart::Before) {
        ResolveOutcome::Pending(s) => s,
        ResolveOutcome::Resolved(_) => panic!("expected Pending"),
    };
    assert_eq!(
        (id_a.sequence, id_b.sequence, id_c.sequence, id_a2.sequence),
        (0, 1, 2, 3)
    );
    assert!(
        [id_a, id_b, id_c, id_a2]
            .iter()
            .all(|id| id.page_index == 0),
        "no page boundary crossed in this test — every slot stays on page 0"
    );
    assert_eq!(reg.pending_slots.len(), 4);

    // Register #a only. #b and #c are still absent.
    reg.register(
        Symbol::new("a"),
        make_info(&[("chapter", &[7])], &[(ContentPart::Before, "prefix")]),
    );

    let resolutions = reg.flush_pending();

    // Two slots resolved (both for #a), two retained (for #b and #c).
    assert_eq!(resolutions.len(), 2);
    assert_eq!(reg.pending_slots.len(), 2);

    // Order preservation: resolutions came out in original queue order —
    // seq 0 (#a counter) before seq 3 (#a text).
    assert_eq!(
        resolutions[0],
        PendingResolution {
            slot_id: id_a,
            value: Some("7".to_owned()),
        }
    );
    assert_eq!(
        resolutions[1],
        PendingResolution {
            slot_id: id_a2,
            value: Some("prefix".to_owned()),
        }
    );

    // Retained slots keep their original ordering (seq 1 before seq 2).
    assert_eq!(reg.pending_slots[0].id, id_b);
    assert_eq!(reg.pending_slots[1].id, id_c);

    // Registering #b later resolves it on the next flush; #c stays.
    reg.register(Symbol::new("b"), make_info(&[("section", &[1, 2])], &[]));
    let resolutions2 = reg.flush_pending();
    assert_eq!(resolutions2.len(), 1);
    assert_eq!(resolutions2[0].slot_id, id_b);
    assert_eq!(resolutions2[0].value.as_deref(), Some("1-2"));
    assert_eq!(reg.pending_slots.len(), 1);
    assert_eq!(reg.pending_slots[0].id, id_c);
}

#[test]
fn flush_pending_counters_variant_joins_stack() {
    // Pending path for target-counters must remember `separator` and
    // `style`, not just the counter `name` — regression check for the
    // TargetRequest::Counters variant fields.
    let mut reg = TargetRegistry::default();
    let out = reg.resolve_target_counters(
        "#sec-1-2-3",
        Symbol::new("section"),
        "-",
        CounterStyle::Decimal,
    );
    assert!(matches!(out, ResolveOutcome::Pending(_)));

    reg.register(
        Symbol::new("sec-1-2-3"),
        make_info(&[("section", &[1, 2, 3])], &[]),
    );
    let resolutions = reg.flush_pending();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].value.as_deref(), Some("1-2-3"));
}

// ── TargetSlotId pairing / begin_page ────

#[test]
fn begin_page_resets_sequence_and_advances_page_index() {
    // Design §7.6 "Slot ID の安定性保証": sequence is page-local
    // (0-indexed within the page), so a page-boundary call must restart
    // the local count. The same local sequence value on two different
    // pages must therefore produce two distinct TargetSlotIds.
    let mut reg = TargetRegistry::default();

    let out_page0 = reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal);
    let id_page0 = match out_page0 {
        ResolveOutcome::Pending(id) => id,
        ResolveOutcome::Resolved(_) => panic!("expected Pending"),
    };
    assert_eq!(
        id_page0,
        TargetSlotId {
            page_index: 0,
            sequence: 0,
        }
    );

    reg.begin_page(1);
    let out_page1 = reg.resolve_target_counter("#b", Symbol::new("chapter"), CounterStyle::Decimal);
    let id_page1 = match out_page1 {
        ResolveOutcome::Pending(id) => id,
        ResolveOutcome::Resolved(_) => panic!("expected Pending"),
    };

    assert_eq!(
        id_page1,
        TargetSlotId {
            page_index: 1,
            sequence: 0,
        },
        "begin_page must reset the local sequence counter to 0"
    );
    assert_eq!(
        id_page0.sequence, id_page1.sequence,
        "both slots are each page's first dispatch — same local sequence"
    );
    assert_ne!(
        id_page0, id_page1,
        "same local sequence on different pages must yield distinct TargetSlotIds"
    );
}

#[test]
fn begin_page_same_index_is_a_no_op() {
    // A redundant begin_page call for the page already in progress must
    // not reset the count out from under slots already queued this
    // page — only an actual page transition resets `next_sequence`.
    let mut reg = TargetRegistry::default();
    reg.begin_page(0); // already page 0 — no-op
    let out_a = reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal);
    reg.begin_page(0); // redundant same-page call — still a no-op
    let out_b = reg.resolve_target_counter("#b", Symbol::new("chapter"), CounterStyle::Decimal);

    let (id_a, id_b) = match (out_a, out_b) {
        (ResolveOutcome::Pending(a), ResolveOutcome::Pending(b)) => (a, b),
        _ => panic!("expected both Pending"),
    };
    assert_eq!(id_a.page_index, 0);
    assert_eq!(id_b.page_index, 0);
    assert_eq!(
        (id_a.sequence, id_b.sequence),
        (0, 1),
        "redundant same-index begin_page must not reset the in-progress page's sequence"
    );
}

#[test]
#[should_panic(expected = "page_index must be monotonically non-decreasing")]
fn begin_page_backward_call_panics_instead_of_duplicating_slot_id() {
    // Without a monotonicity guard, a backward begin_page() call resets
    // next_sequence to 0 while an earlier page's slot is still
    // unresolved in pending_slots (unresolved slots are retained across
    // flushes forever — see flush_pending_retains_unregistered_slots).
    // The very next dispatch on the revisited page would then mint a
    // TargetSlotId byte-identical to the still-pending one, breaking
    // the uniqueness guarantee TargetSlotId exists to provide
    // (crate::error::TargetSlotId's doc comment). This must now panic
    // instead of silently corrupting pending_slots.
    let mut reg = TargetRegistry::default();

    // Step 1: begin_page(0), dispatch an unresolved target — mints
    // TargetSlotId{page_index:0, sequence:0}, retained in pending_slots
    // (its fragment "#never-registered" is never registered).
    let out_page0 = reg.resolve_target_counter(
        "#never-registered",
        Symbol::new("chapter"),
        CounterStyle::Decimal,
    );
    let id_page0 = match out_page0 {
        ResolveOutcome::Pending(id) => id,
        ResolveOutcome::Resolved(_) => panic!("expected Pending"),
    };
    assert_eq!(
        id_page0,
        TargetSlotId {
            page_index: 0,
            sequence: 0,
        }
    );
    assert_eq!(reg.pending_slots.len(), 1, "slot from page 0 still queued");

    // Step 2: begin_page(1), more dispatches on page 1.
    reg.begin_page(1);
    let _ = reg.resolve_target_counter("#b", Symbol::new("chapter"), CounterStyle::Decimal);

    // Step 3: begin_page(0) again — a backward call (convergence
    // re-run / two-pass layout walk / driver bug). This must panic
    // rather than reset next_sequence out from under the still-pending
    // page-0 slot from step 1.
    reg.begin_page(0);
}

#[test]
fn target_slot_id_is_stable_across_repeated_construction() {
    // the same (page_index, sequence) pair twice must compare equal and
    // hash identically — Consumer patch tables key off this.
    let a = TargetSlotId {
        page_index: 2,
        sequence: 5,
    };
    let b = TargetSlotId {
        page_index: 2,
        sequence: 5,
    };
    assert_eq!(a, b);

    let mut set = std::collections::HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}

// ── ContentComponent wire-through (conversion path) ─────

#[test]
fn resolve_content_component_drives_target_counter() {
    let cc = ContentComponent::TargetCounter {
        url: "#chapter-3".to_owned(),
        name: SmolStr::new("chapter"),
        style: CounterStyle::Decimal,
    };

    let mut reg = TargetRegistry::default();
    reg.register(
        Symbol::new("chapter-3"),
        make_info(&[("chapter", &[1, 2, 3])], &[]),
    );

    let outcome = resolve_content_component(&mut reg, &cc)
        .expect("target-* variant should return Some(outcome)");
    assert_eq!(outcome, ResolveOutcome::Resolved("3".to_owned()));
}

#[test]
fn resolve_content_component_drives_target_counters() {
    let cc = ContentComponent::TargetCounters {
        url: "#sec-1-2".to_owned(),
        name: SmolStr::new("section"),
        separator: ".".to_owned(),
        style: CounterStyle::Decimal,
    };

    let mut reg = TargetRegistry::default();
    reg.register(
        Symbol::new("sec-1-2"),
        make_info(&[("section", &[1, 2])], &[]),
    );

    let outcome = resolve_content_component(&mut reg, &cc)
        .expect("target-* variant should return Some(outcome)");
    assert_eq!(outcome, ResolveOutcome::Resolved("1.2".to_owned()));
}

#[test]
fn resolve_content_component_drives_target_text() {
    let cc = ContentComponent::TargetText {
        url: "#h1".to_owned(),
        part: ContentPart::Content,
    };

    let mut reg = TargetRegistry::default();
    reg.register(
        Symbol::new("h1"),
        make_info(&[], &[(ContentPart::Content, "Hello")]),
    );

    let outcome = resolve_content_component(&mut reg, &cc)
        .expect("target-* variant should return Some(outcome)");
    assert_eq!(outcome, ResolveOutcome::Resolved("Hello".to_owned()));
}

#[test]
fn resolve_content_component_non_fragment_url_falls_back_to_empty() {
    // Every target-* variant driven through resolve_content_component
    // must inherit the non-fragment fallback (unresolvable per-document,
    // no pending slot queued).
    let mut reg = TargetRegistry::default();

    let counter_cc = ContentComponent::TargetCounter {
        url: "https://example.com/".to_owned(),
        name: SmolStr::new("chapter"),
        style: CounterStyle::Decimal,
    };
    assert_eq!(
        resolve_content_component(&mut reg, &counter_cc),
        Some(ResolveOutcome::Resolved(String::new()))
    );

    let counters_cc = ContentComponent::TargetCounters {
        url: "http://elsewhere/".to_owned(),
        name: SmolStr::new("section"),
        separator: ".".to_owned(),
        style: CounterStyle::Decimal,
    };
    assert_eq!(
        resolve_content_component(&mut reg, &counters_cc),
        Some(ResolveOutcome::Resolved(String::new()))
    );

    let text_cc = ContentComponent::TargetText {
        url: "".to_owned(),
        part: ContentPart::Content,
    };
    assert_eq!(
        resolve_content_component(&mut reg, &text_cc),
        Some(ResolveOutcome::Resolved(String::new()))
    );

    // All three fallbacks must have been synchronous — no pending slots
    // must have been queued for the unresolvable URLs.
    assert!(reg.pending_slots.is_empty());
}

#[test]
fn resolve_content_component_returns_none_for_non_target_variants() {
    // Every non-target ContentComponent variant must decline the resolve
    // — the pending queue must not grow for Literal / Counter / etc.
    // Guards against future non_exhaustive variants: catch-all in
    // resolve_content_component keeps them None-by-default (safe:
    // resolve pass ignores them, other passes will handle them).
    let mut reg = TargetRegistry::default();

    let literal = ContentComponent::Literal(SmolStr::new("hello"));
    assert!(resolve_content_component(&mut reg, &literal).is_none());

    let counter = ContentComponent::Counter {
        name: SmolStr::new("chapter"),
        style: CounterStyle::Decimal,
    };
    assert!(resolve_content_component(&mut reg, &counter).is_none());

    let attr = ContentComponent::Attr {
        name: SmolStr::new("href"),
    };
    assert!(resolve_content_component(&mut reg, &attr).is_none());

    // No non-target dispatch should have queued a pending slot.
    assert!(reg.pending_slots.is_empty());
}
