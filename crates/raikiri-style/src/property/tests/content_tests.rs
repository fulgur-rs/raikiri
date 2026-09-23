//! Tests for the generated-content property parsers in `parse/content.rs`.

use super::*;

// ── counter-* (CSS Lists 3 §4) ──

// `PropertyValue::Counter*(Arc<Vec<..>>)` に wrap したため、
// literal test 比較用に Arc<Vec<..>> を返す helper に切り替え
// (content/string_set helper と同 pattern)。
fn counter_pairs(pairs: &[(&str, i32)]) -> Arc<Vec<(SmolStr, i32)>> {
    Arc::new(
        pairs
            .iter()
            .map(|(name, value)| (SmolStr::new(name), *value))
            .collect(),
    )
}

#[test]
fn counter_reset_single_name_defaults_to_zero() {
    // spec: reset の default は 0
    assert_eq!(
        parse("chapter", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[(
            "chapter", 0
        )])))
    );
}

#[test]
fn counter_reset_multiple_names_with_mixed_ints() {
    // 2 番目に integer が付く → 1 番目は default 0、2 番目は 3
    assert_eq!(
        parse("chapter section 3", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[
            ("chapter", 0),
            ("section", 3)
        ])))
    );
}

#[test]
fn counter_reset_none_returns_empty_vec() {
    // spec: `none` は空リストと同等 (top-level alternative)
    // empty case は shared Arc slot (`empty_counter_entries`) を使う。
    assert_eq!(
        parse("none", "counter-reset"),
        Some(PropertyValue::CounterReset(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_rejects_number_first() {
    // 先頭が number → ident が来るまで peel できず empty → None (drop)
    // spec §4: `<counter-name> = <custom-ident>` (数値は counter-name ではない)
    assert_eq!(parse("123 abc", "counter-reset"), None);
}

#[test]
fn counter_increment_single_name_defaults_to_one() {
    // spec: increment の default は 1
    assert_eq!(
        parse("chapter", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[(
            "chapter", 1
        )])))
    );
}

#[test]
fn counter_increment_mixed_int_and_default() {
    // `chapter 2 section` → chapter=2、section=default(1)
    assert_eq!(
        parse("chapter 2 section", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[
            ("chapter", 2),
            ("section", 1)
        ])))
    );
}

#[test]
fn counter_increment_accepts_negative_integer() {
    // spec §4: <integer> — negative も valid (counter を decrement する用途)
    assert_eq!(
        parse("chapter -1", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[(
            "chapter", -1
        )])))
    );
}

#[test]
fn counter_increment_none_returns_empty_vec() {
    // empty case は shared Arc slot を使う。
    assert_eq!(
        parse("none", "counter-increment"),
        Some(PropertyValue::CounterIncrement(empty_counter_entries()))
    );
}

#[test]
fn counter_set_defaults_to_zero() {
    // spec: set の default は 0
    assert_eq!(
        parse("page 5 note", "counter-set"),
        Some(PropertyValue::CounterSet(counter_pairs(&[
            ("page", 5),
            ("note", 0)
        ])))
    );
}

#[test]
fn counter_set_none_returns_empty_vec() {
    // empty case は shared Arc slot を使う。
    assert_eq!(
        parse("none", "counter-set"),
        Some(PropertyValue::CounterSet(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_is_case_insensitive_on_none() {
    // CSS spec: keyword `none` は ASCII case-insensitive
    // empty case は shared Arc slot を使う。
    assert_eq!(
        parse("NONE", "counter-reset"),
        Some(PropertyValue::CounterReset(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_accepts_inherit_marker_and_rejects_other_reserved_names() {
    // `counter-reset: inherit` is retained as a page-context marker;
    // the other CSS-wide keywords are not valid counter names.
    assert_eq!(
        parse("inherit", "counter-reset"),
        Some(PropertyValue::CounterResetInherit)
    );
    assert_eq!(parse("initial", "counter-reset"), None);
    assert_eq!(parse("unset", "counter-reset"), None);
    assert_eq!(parse("revert", "counter-reset"), None);
    assert_eq!(parse("default", "counter-reset"), None);
}

#[test]
fn counter_reset_accepts_negative_integer() {
    // CSS Values 3 §4.2 "Integers: the <integer> type"
    // (https://www.w3.org/TR/css-values-3/#integers): <integer> は負値を含む。
    // increment だけでなく reset / set も同一 grammar。
    assert_eq!(
        parse("chapter -5", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[(
            "chapter", -5
        )])))
    );
}

#[test]
fn counter_set_accepts_negative_integer() {
    // 同上 (parity with reset/increment negative-integer coverage)。
    assert_eq!(
        parse("page -3", "counter-set"),
        Some(PropertyValue::CounterSet(counter_pairs(&[("page", -3)])))
    );
}

// ── content property (CSS Content 3 §2) ──
//
// task の verification items は spec-derived。task 記述の
// `raikiri_traits::ContentValueItem` は下流 (raikiri-dom) mapping 先。
// raikiri-style は raikiri-traits に依存しない leaf crate
// のため、counter-* precedent に倣い local `ContentComponent` を emit
// する (原則 1: 前例主義)。Symbol → SmolStr、Url → String へ substitution。

#[test]
fn content_parse_string_function() {
    // Verification 1: content: string(my_str)
    // → ContentComponent::String { name: "my_str", fetch: default (First) }
    let items = content_items("string(my_str)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::String {
            name: SmolStr::new("my_str"),
            fetch: StringFetchMode::First,
        }
    );
}

#[test]
fn content_parse_counter_function() {
    // Verification 2: content: counter(chapter)
    // → ContentComponent::Counter { name: "chapter", style: default (Decimal) }
    let items = content_items("counter(chapter)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_counters_function() {
    // Verification 3: content: counters(section, ".")
    // → ContentComponent::Counters { name, separator: ".", style: default }
    let items = content_items(r#"counters(section, ".")"#);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Counters {
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_counter_function() {
    // Verification 4: content: target-counter(url("#anchor"), page)
    // → ContentComponent::TargetCounter { url: "#anchor", name: "page", style: default }
    let items = content_items(r##"target-counter(url("#anchor"), page)"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetCounter {
            url: String::from("#anchor"),
            name: SmolStr::new("page"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_counters_function() {
    // Verification 5: content: target-counters(url("#anchor"), section, ".")
    // → ContentComponent::TargetCounters { url, name, separator, style: default }
    let items = content_items(r##"target-counters(url("#anchor"), section, ".")"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetCounters {
            url: String::from("#anchor"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_text_first_letter() {
    // Verification 6: content: target-text(url("#anchor"), first-letter)
    // → ContentComponent::TargetText { url, part: ContentPart::FirstLetter }
    //
    // NB: task description の "content-first-letter" は spec (§2.6.3
    // `[ content | before | after | first-letter ]?`) と食い違うため、
    // spec-correct な `first-letter` を採用 (task 側の記述が誤りと
    // 判明したための訂正)。
    let items = content_items(r##"target-text(url("#anchor"), first-letter)"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetText {
            url: String::from("#anchor"),
            part: ContentPart::FirstLetter,
        }
    );
}

#[test]
fn content_parse_attr_function() {
    // Verification 7: content: attr(href) → ContentComponent::Attr { name: "href" }
    let items = content_items("attr(href)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Attr {
            name: SmolStr::new("href"),
        }
    );
}

#[test]
fn content_parse_attr_untyped_fallbacks() {
    let items = content_items(r#"attr(missing, "Fallback value") attr(missing, invalid)"#);
    assert_eq!(
        items,
        vec![
            ContentComponent::AttrFallback {
                name: SmolStr::new("missing"),
                fallback: Some(SmolStr::new("Fallback value")),
            },
            ContentComponent::AttrFallback {
                name: SmolStr::new("missing"),
                fallback: None,
            },
        ]
    );
}

#[test]
fn content_parse_literal_string() {
    // Verification 8: content: "hello" → ContentComponent::Literal("hello")
    let items = content_items(r#""hello""#);
    assert_eq!(
        items,
        vec![ContentComponent::Literal(SmolStr::new("hello"))]
    );
}

#[test]
fn content_parse_mixed_sequence_preserves_order() {
    // Verification 9: content: "Chapter " counter(chapter) ": " string(chapter_title)
    // → 4-item Vec in order
    let items = content_items(r#""Chapter " counter(chapter) ": " string(chapter_title)"#);
    assert_eq!(items.len(), 4, "expected 4 items, got {items:?}");
    assert_eq!(
        items[0],
        ContentComponent::Literal(SmolStr::new("Chapter "))
    );
    assert_eq!(
        items[1],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
    assert_eq!(items[2], ContentComponent::Literal(SmolStr::new(": ")));
    assert_eq!(
        items[3],
        ContentComponent::String {
            name: SmolStr::new("chapter_title"),
            fetch: StringFetchMode::First,
        }
    );
}

// ── content property: image / contents / <quote> / leader() (CSS Content 3
// §2.2 / §2.3 / §2.4.2 / §2.5.1 — under-accept fix、CssContent3 mode arm) ──

#[test]
fn content_parse_image_url_quoted_form() {
    // `<image>` の `<url>` alternative、`url("...")` (quoted) form。
    let items = content_items(r#"url("cat.png")"#);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Image {
            url: String::from("cat.png"),
        }
    );
}

#[test]
fn content_parse_image_url_unquoted_form() {
    // `<image>` の `<url>` alternative、`url(...)` (unquoted url-token) form。
    let items = content_items("url(cat.png)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Image {
            url: String::from("cat.png"),
        }
    );
}

#[test]
fn content_bare_string_is_still_literal_not_image() {
    // Regression check: `<image>` production は `<url> | <gradient>` のみで
    // bare `<string>` を含まない (target-* の `[<string>|<url>]` とは別
    // grammar)。`expect_url` は quoted string 単体を受理しないため
    // `content: "cat.png"` は Literal のまま — Image への誤変換防止。
    let items = content_items(r#""cat.png""#);
    assert_eq!(
        items,
        vec![ContentComponent::Literal(SmolStr::new("cat.png"))]
    );
}

#[test]
fn content_parse_contents_keyword() {
    // CSS Content 3 §2.3 "Elemental Content: the contents keyword"。
    let items = content_items("contents");
    assert_eq!(items, vec![ContentComponent::Contents]);
}

#[test]
fn content_contents_keyword_is_case_insensitive() {
    let items = content_items("CoNtEnTs");
    assert_eq!(items, vec![ContentComponent::Contents]);
}

#[test]
fn content_parse_quote_keywords() {
    // CSS Content 3 §2.4.2 `<quote> = open-quote | close-quote |
    // no-open-quote | no-close-quote` の 4 keyword 全数検証。
    assert_eq!(
        content_items("open-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
    );
    assert_eq!(
        content_items("close-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::CloseQuote)]
    );
    assert_eq!(
        content_items("no-open-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::NoOpenQuote)]
    );
    assert_eq!(
        content_items("no-close-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::NoCloseQuote)]
    );
}

#[test]
fn content_quote_keyword_is_case_insensitive() {
    let items = content_items("OPEN-QUOTE");
    assert_eq!(
        items,
        vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
    );
}

#[test]
fn content_parse_leader_dotted_solid_space_keywords() {
    // CSS Content 3 §2.5.1 `<leader-type> = dotted | solid | space | <string>`。
    assert_eq!(
        content_items("leader(dotted)"),
        vec![ContentComponent::Leader(LeaderType::Dotted)]
    );
    assert_eq!(
        content_items("leader(solid)"),
        vec![ContentComponent::Leader(LeaderType::Solid)]
    );
    assert_eq!(
        content_items("leader(space)"),
        vec![ContentComponent::Leader(LeaderType::Space)]
    );
}

#[test]
fn content_parse_leader_custom_string() {
    let items = content_items(r#"leader(".~.")"#);
    assert_eq!(
        items,
        vec![ContentComponent::Leader(LeaderType::String(SmolStr::new(
            ".~."
        )))]
    );
}

#[test]
fn content_leader_is_case_insensitive() {
    let items = content_items("LEADER(DOTTED)");
    assert_eq!(items, vec![ContentComponent::Leader(LeaderType::Dotted)]);
}

#[test]
fn content_leader_rejects_missing_argument() {
    // spec production `leader( <leader-type> )` に `?` が無いため引数必須。
    // bare `leader()` は spec-invalid → declaration drop。
    assert_eq!(parse("leader()", "content"), None);
}

#[test]
fn content_leader_rejects_unknown_keyword() {
    assert_eq!(parse("leader(bogus)", "content"), None);
}

#[test]
fn content_rejects_unknown_bare_keyword() {
    // `parse_content_bare_keyword` の 5 keyword (`contents` / 4 `<quote>`)
    // いずれにも一致しない ident は catch-all `_ => None` に落ちる —
    // items 0 → declaration drop (単独 token の場合)。
    assert_eq!(parse("bogus", "content"), None);
}

#[test]
fn content_unknown_bare_keyword_mid_list_stops_items_and_leaves_leftover() {
    // 認識済み item (`counter(chapter)`) の後に未知 ident が来た場合、
    // items+ loop は unknown token で break する (catch-all の break 経路)。
    // caller (rule.rs) の `expect_exhausted` 相当は `parse` helper では
    // 経由しないため、本 helper 経由では 1-item 到達で観測できる — leftover
    // 自体の drop 挙動は既存 `content_rejects_unknown_function` /
    // `string_set_accepts_missing_comma_single_leftover_entry` と同じ
    // break-then-leftover pattern の non-regression check。
    let items = content_items("counter(chapter) bogus");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

#[test]
fn content_parse_mixed_sequence_with_new_alternatives() {
    // image / contents / quote / leader を既存 alternative と混在させ、
    // 順序が保持されることを検証。
    let items =
        content_items(r#"open-quote "term" close-quote leader(dotted) url("icon.png") contents"#);
    assert_eq!(
        items,
        vec![
            ContentComponent::Quote(QuoteKeyword::OpenQuote),
            ContentComponent::Literal(SmolStr::new("term")),
            ContentComponent::Quote(QuoteKeyword::CloseQuote),
            ContentComponent::Leader(LeaderType::Dotted),
            ContentComponent::Image {
                url: String::from("icon.png"),
            },
            ContentComponent::Contents,
        ]
    );
}

// ── string-set narrow <content-list> gate: image / contents / quote /
// leader() (CSS GCPM 3 §1.1.1 L82) ──
//
// GCPM 3 §1.1.1 narrow list には `<image>` / `contents` / `<quote>` /
// `leader()` のいずれも含まれない (既存の string_set_rejects_* group と
// 同じ rationale — sibling test 群と揃えて 1 declaration = 1 rejection の
// check にする)。

#[test]
fn string_set_rejects_image_url() {
    assert_eq!(parse(r#"title url("a.png")"#, "string-set"), None);
}

#[test]
fn string_set_rejects_contents_keyword() {
    assert_eq!(parse("title contents", "string-set"), None);
}

#[test]
fn string_set_rejects_quote_keyword() {
    assert_eq!(parse("title open-quote", "string-set"), None);
}

#[test]
fn string_set_rejects_leader_fn() {
    assert_eq!(parse("title leader(dotted)", "string-set"), None);
}

// ── content property edge cases (spec-derived、guard rails) ──

#[test]
fn content_normal_returns_empty_list() {
    // spec §1: `normal` は「content が明示されない場合と同じ」= 空 list として保持。
    // pseudo-element generation 判断は下流で行う。
    assert_eq!(
        parse("normal", "content"),
        Some(PropertyValue::Content(empty_content_list()))
    );
}

#[test]
fn content_none_keeps_a_suppression_sentinel() {
    assert_eq!(
        parse("none", "content"),
        Some(PropertyValue::Content(Arc::new(vec![
            ContentComponent::None
        ])))
    );
}

#[test]
fn content_string_with_fetch_last_keyword() {
    // spec §2.7.2 の string() 第 2 引数 keyword を全て受理することを smoke で pin。
    let items = content_items("string(head, last)");
    assert_eq!(
        items,
        vec![ContentComponent::String {
            name: SmolStr::new("head"),
            fetch: StringFetchMode::Last,
        }]
    );
}

#[test]
fn content_target_text_default_part_is_content() {
    // target-text() の第 2 引数省略時、raikiri は ContentPart::Content を
    // フォールバック値として使う (根拠は spec の "default" 宣言ではない —
    // CSS Content 3 §2.6.3 は第 2 引数省略時の値を規定していない)。
    let items = content_items(r##"target-text(url("#a"))"##);
    assert_eq!(
        items,
        vec![ContentComponent::TargetText {
            url: String::from("#a"),
            part: ContentPart::Content,
        }]
    );
}

#[test]
fn content_counter_rejects_none_name() {
    // spec CSS Lists 3 §4 <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>:
    // "A <counter-name> name cannot match the keyword `none`; such an identifier
    // is invalid as a <counter-name>". §4.7 counter() の first argument が
    // <counter-name> production のため `counter(none)` は declaration drop。
    // counter-reset/increment/set (property.rs 既存) と一貫、Chrome/FF と一致。
    assert_eq!(parse("counter(none)", "content"), None);
}

#[test]
fn content_counters_rejects_none_name() {
    // spec CSS Lists 3 §4 / §4.7: counters() の first argument も
    // <counter-name> production、`none` は invalid。
    assert_eq!(parse(r#"counters(none, ".")"#, "content"), None);
}

#[test]
fn content_counter_with_named_style_preserves_ident() {
    // spec CSS Lists 3 §4.7: 第 2 引数 `<counter-style>` は decimal 以外の
    // named style も受ける。下流 (raikiri-dom) が解釈するため raw ident 保持。
    let items = content_items("counter(chapter, upper-alpha)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Named(SmolStr::new("upper-alpha")),
        }]
    );
}

#[test]
fn content_rejects_unknown_function() {
    // 未知 function は認識できず、items 開始 token として peel 失敗。
    // 先頭 token が unknown function だと empty items → None (drop)。
    assert_eq!(parse("bogus(x)", "content"), None);
}

#[test]
fn content_case_insensitive_function_name() {
    // spec: function name は ASCII case-insensitive。
    let items = content_items("COUNTER(chapter)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

#[test]
fn content_key_maps_to_content_property_key() {
    // PropertyValue::Content → PropertyKey::Content (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* と同じ pattern)。
    let cv = PropertyValue::Content(empty_content_list());
    assert_eq!(cv.key(), PropertyKey::Content);
}

// ── parse_optional_counter_style trailing-comma strict reject ──
//
// CSS Lists 3 §4.7 `counter(<counter-name>, <counter-style>?)` /
// CSS Content 3 §2.6.1-2 `target-counter()` / `target-counters()` は
// `<counter-style>?` — `,` を先行させる時は ident 必須。trailing-comma
// (`counter(chapter,)` 等) は spec-invalid → declaration ごと drop すべき。
// sibling `parse_string_fetch` / `parse_content_part` は既に strict `?`
// propagation、`parse_optional_counter_style` のみ silent Decimal fallback
// していた regression を check する。

#[test]
fn content_counter_rejects_trailing_comma() {
    // `counter(chapter,)` — comma 消費後に ident 不在。spec-invalid、
    // declaration drop = None (Chrome/Firefox と同挙動)。
    assert_eq!(parse("counter(chapter,)", "content"), None);
}

#[test]
fn content_counters_rejects_trailing_comma() {
    // `counters(chapter, ".",)` — separator string 後の trailing comma。
    assert_eq!(parse(r#"counters(chapter, ".",)"#, "content"), None);
}

#[test]
fn content_target_counter_rejects_trailing_comma() {
    // `target-counter(url("#a"), page,)` — name 後の trailing comma。
    // target-counter/target-counters は parse_optional_counter_style を
    // 経由 (parse_target_counter_fn / parse_target_counters_fn) するため同じ pattern で drop。
    assert_eq!(
        parse(r##"target-counter(url("#a"), page,)"##, "content"),
        None
    );
}

#[test]
fn content_target_counters_rejects_trailing_comma() {
    // `target-counters(url("#a"), section, ".",)` — separator 後の trailing。
    assert_eq!(
        parse(r##"target-counters(url("#a"), section, ".",)"##, "content"),
        None
    );
}

#[test]
fn content_string_rejects_trailing_comma() {
    // 対照実験 (現行 strict の維持確認): `string(foo,)` は
    // `parse_string_fetch` が `?` 経由で伝播、既に None。
    assert_eq!(parse("string(foo,)", "content"), None);
}

#[test]
fn content_target_text_rejects_trailing_comma() {
    // 対照実験: `target-text(url("#a"),)` は `parse_content_part` が
    // `?` 経由で伝播、既に None。
    assert_eq!(parse(r##"target-text(url("#a"),)"##, "content"), None);
}

#[test]
fn content_counter_accepts_bare_default() {
    // `counter(chapter)` — trailing comma 無しの正常 case、Decimal default
    // で Some を返す (silent fallback を strict にしても正常 path は変えない)。
    let items = content_items("counter(chapter)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

// ── string-set (CSS GCPM 3 §1.1.1) ──
//
// grammar: `none | [ <custom-ident> <content-list> ]#` — 各 entry は
// (name, content-list) pair、`ContentComponent` + `parse_content_list_items`
// を reuse。task description の "4-item Vec" は entry name の分を content 側に
// 誤って含めた結果、実態は 3-item (name は tuple の第 1 要素)。

fn string_set_entries(source: &str) -> Vec<(SmolStr, Vec<ContentComponent>)> {
    match parse(source, "string-set") {
        // PropertyValue::StringSet(Arc<Vec<..>>)、content_items と同 pattern。
        Some(PropertyValue::StringSet(v)) => (*v).clone(),
        other => panic!("expected PropertyValue::StringSet, got {other:?}"),
    }
}

#[test]
fn string_set_single_entry_with_literal() {
    // Verification 1: string-set: my_str "hello"
    // → `[(SmolStr("my_str"), [Literal("hello")])]`
    let entries = string_set_entries(r#"my_str "hello""#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("my_str"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("hello"))]
    );
}

#[test]
fn string_set_mixed_content_list_preserves_order() {
    // Verification 2:
    // string-set: chapter_title counter(chapter) ": " attr(title)
    //
    // 先頭 `chapter_title` は entry name (tuple 第 1 要素)。content-list は
    // 残りの `counter(chapter) ": " attr(title)` = 3 items。
    //
    // NB: 原 test は末尾に `string(chapter_title)` を置いていたが、
    // GCPM 3 §1.1.1 narrow list は `string()` function を含まないため
    // `attr()` (GCPM narrow list の 5 alt の 1 つ) に
    // swap。テストの主意 (mixed content-list の order 保持) は保つ。
    let entries = string_set_entries(r#"chapter_title counter(chapter) ": " attr(title)"#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("chapter_title"));
    assert_eq!(entries[0].1.len(), 3);
    assert_eq!(
        entries[0].1[0],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
    assert_eq!(
        entries[0].1[1],
        ContentComponent::Literal(SmolStr::new(": "))
    );
    assert_eq!(
        entries[0].1[2],
        ContentComponent::Attr {
            name: SmolStr::new("title"),
        }
    );
}

#[test]
fn string_set_comma_separated_multi_entry() {
    // Verification 3: string-set: a "x", b "y" → 2 entries
    let entries = string_set_entries(r#"a "x", b "y""#);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, SmolStr::new("a"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("x"))]
    );
    assert_eq!(entries[1].0, SmolStr::new("b"));
    assert_eq!(
        entries[1].1,
        vec![ContentComponent::Literal(SmolStr::new("y"))]
    );
}

#[test]
fn string_set_none_returns_empty_vec() {
    // spec §1.1.1: top-level `none` = empty list
    assert_eq!(
        parse("none", "string-set"),
        Some(PropertyValue::StringSet(empty_string_set_entries()))
    );
}

#[test]
fn string_set_rejects_reserved_css_wide_keyword_as_name() {
    // spec §1.1.1 + CSS Values 4 §4.2
    // <https://www.w3.org/TR/css-values-4/#custom-idents>:
    // `<custom-ident>` は CSS-wide keyword 除外。
    // 先頭 ident が `inherit` → try_parse rewind で entries 空 → None。
    //
    // NB: 先頭が `none` の場合は top-level alternative の branch を先に
    // 通って `Some(empty)` を返し、leftover は下流 `expect_exhausted` で
    // declaration drop (rule.rs level)。この case は parse_value 単体では
    // 検証しない。
    assert_eq!(parse("inherit \"x\"", "string-set"), None);
    assert_eq!(parse("initial \"x\"", "string-set"), None);
    assert_eq!(parse("unset \"x\"", "string-set"), None);
    assert_eq!(parse("revert \"x\"", "string-set"), None);
    assert_eq!(parse("default \"x\"", "string-set"), None);
}

#[test]
fn string_set_rejects_name_without_content_list() {
    // spec §1.1.1 + Content 3 §2: `<content-list>` は 1+ items 必須。
    // name だけで items 0 → declaration drop (None)。
    assert_eq!(parse("my_str", "string-set"), None);
}

#[test]
fn string_set_is_case_insensitive_on_none() {
    // CSS spec: keyword `none` は ASCII case-insensitive
    assert_eq!(
        parse("NONE", "string-set"),
        Some(PropertyValue::StringSet(empty_string_set_entries()))
    );
}

#[test]
fn string_set_key_maps_to_string_set_property_key() {
    // PropertyValue::StringSet → PropertyKey::StringSet (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* / content と同じ pattern)。
    let v = PropertyValue::StringSet(empty_string_set_entries());
    assert_eq!(v.key(), PropertyKey::StringSet);
}

// ── string-set trailing-comma strict reject ──
//
// `#` (comma-separated multiplier、CSS Values 4 §2.3
// <https://www.w3.org/TR/css-values-4/#mult-comma>) は trailing comma を
// 許容しない。GCPM 3 §1.1.1 <string-set-value> = `[ <custom-ident>
// <content-list> ]#` は entry 間 comma 必須 + trailing comma 禁止。
//
// 初期実装は separator loop で `try_parse(expect_comma).is_err() {
// break }` していたため、trailing comma を silently 受理していた (comma を
// consume 後 next iteration で name parse fail → break → 既存 entries を
// Some で返す)。同じ principle の `.ok()?` propagation で strict 化。

#[test]
fn string_set_rejects_trailing_comma_single_entry() {
    // `string-set: a "x",` → trailing comma → declaration drop。
    // pre-fix は Some(`[(a, [Literal("x")])]`) を silently 返していた。
    assert_eq!(parse(r#"a "x","#, "string-set"), None);
}

#[test]
fn string_set_rejects_trailing_comma_two_entries() {
    // `string-set: a "x", b "y",` → trailing comma → declaration drop。
    // 内部 comma 1 個は valid separator、末尾 comma のみが `#` 違反。
    assert_eq!(parse(r#"a "x", b "y","#, "string-set"), None);
}

#[test]
fn string_set_rejects_trailing_comma_three_entries() {
    // 3 entries + trailing comma — chain 越しの一貫 strict reject を pin。
    assert_eq!(parse(r#"a "x", b "y", c "z","#, "string-set"), None);
}

#[test]
fn string_set_rejects_missing_entry_after_comma() {
    // `string-set: a "x", b` → comma 後 name は取れるが `<content-list>`
    // が 0 items (`parse_content_list_items` empty) → declaration drop。
    // trailing-comma 系とは reject 経路が異なる (items-empty) 独立 pin。
    assert_eq!(parse(r#"a "x", b"#, "string-set"), None);
}

#[test]
fn string_set_accepts_missing_comma_single_leftover_entry() {
    // `string-set: a "x" b "y"` は separator comma 欠如。iter 1 で
    // (a, `["x"]`) push 後、bottom expect_comma fail → break、leftover
    // `b "y"` は本 helper (parse_value 直呼び、caller expect_exhausted
    // 経由なし) では drop されず 1 entry の Some として観測される。
    // 実 caller (rule.rs) は expect_exhausted で declaration drop する
    // — 本 test は parse_string_set の break exit が Some (`.ok()?`
    // 経路と混同しない) であることを check する目的、trailing-comma fix の
    // non-regression coverage。
    let entries = string_set_entries(r#"a "x" b "y""#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("a"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("x"))]
    );
}

// ── string-set narrow <content-list> gate (CSS GCPM 3 §1.1.1) ──
//
// GCPM 3 §1.1.1 L82 verbatim: <content-list> = [ <string> | <counter()> |
// <counters()> | <content()> | <attr()> ]+ — CSS Content 3 §2 broad list を
// string-set 用に narrower 再定義。`string()` (function、bare literal とは別)
// および `target-counter()` / `target-counters()` / `target-text()` は
// spec grammar に含まれず、`ContentListMode::GcpmStringSet` mode dispatch で
// reject する (parse_content_list_items が 0 items → parse_string_set →
// None → declaration drop、cascade shadow 例
// `p.hi { string-set: title target-counter(url("#x"), page); }` の spec 準拠
// 挙動 = .hi rule drop → parser layer で確認)。
//
// 一方 content property (CssContent3 mode) はこれら全てを引き続き受理する
// (下の content_parse_* 系 check test 群で non-regression 検証)。

#[test]
fn string_set_rejects_string_fn() {
    // GCPM 3 §1.1.1 L82 は `string()` function を narrow list から除外。
    // bare `<string>` literal (`"..."`) と混同しないよう function 側のみ reject。
    assert_eq!(parse("title string(x)", "string-set"), None);
}

#[test]
fn string_set_rejects_target_counter_fn() {
    // GCPM 3 §1.1.1 L82 は `target-counter()` を narrow list から除外。
    // cascade shadow の主要例、declaration drop → cascade で
    // 先行の spec-valid rule が winner になる shape。
    assert_eq!(
        parse(r##"title target-counter(url("#a"), page)"##, "string-set"),
        None
    );
}

#[test]
fn string_set_rejects_target_counters_fn() {
    // GCPM 3 §1.1.1 L82 は `target-counters()` を narrow list から除外。
    assert_eq!(
        parse(
            r##"title target-counters(url("#a"), section, ".")"##,
            "string-set"
        ),
        None
    );
}

#[test]
fn string_set_rejects_target_text_fn() {
    // GCPM 3 §1.1.1 L82 は `target-text()` を narrow list から除外。
    assert_eq!(
        parse(r##"title target-text(url("#a"))"##, "string-set"),
        None
    );
}

// ── content() function (CSS GCPM 3 §1.1.1.1) ──
//
// grammar (spec verbatim, line 758 of TR/css-gcpm-3/, string-set/GCPM3側の
// grammar):
//   content() = content(`[text | before | after | first-letter]`)
// 4 keyword。keyword 省略時は `text` をフォールバック値として使う (根拠は
// GCPM 3 側の spec "default" 宣言ではない — grammar に `?` が無く、"default
// をどう定義するか" 自体が未解決の WG issue として残っている)。GCPM 3
// §1.1.1 の narrow `<content-list>` と CSS Content 3 §2 の broad
// `<content-list>` の両方に対し unconditional に受理されるため、
// string-set および content property 双方の content-list 内で受理される
// (`ContentListMode` mode dispatch 導入後も `content()` arm は両
// mode で unconditional accept)。
//
// CSS Content 3 §2.7.3 は content() を `?` 付き 5 keyword (`marker` 含む)
// で別途定義しており、GCPM 3 §1.1.1.1 と keyword 集合が食い違う。この
// 実装は keyword 集合について GCPM 3 §1.1.1.1 に従うと決めており、content
// property 側でも `marker` は意図的に reject する (詳細・根拠は
// `parse_content_fn` の doc comment 参照)。CSS Content 3 §2.7.3 の
// `marker` keyword は既知の feature gap として残る。
//
// pre-fix reproduction: `string-set: title content(text)` は
// silent drop していた (parse_content_function match arm 欠如 →
// parse_content_list_items break → 0 items → parse_string_set None →
// declaration drop)。arm 追加で Some を返すことを check する。

#[test]
fn string_set_content_text_reproduces_pre_fix_drop() {
    // description の主要 repro case:
    // pre-fix では declaration drop = None、post-fix では
    // (title, `[Content{keyword: Text}]`) を含む Some を返す。
    let entries = string_set_entries("title content(text)");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("title"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_explicit_text_keyword() {
    // §1.1.1.1: `content(text)` は element の string value (bare `content()`
    // のフォールバック値と同じ keyword だが、明示的 keyword 保持で
    // downstream の分岐余地を残す)。
    let items = content_items("content(text)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_default_keyword_on_empty_parens() {
    // §1.1.1.1 の spec 例 `h2 { string-set: heading content() }` (string-set
    // /GCPM3側の文脈) — bare `content()` は `text` をフォールバック値として
    // 使う (根拠は GCPM 3 側の spec "default" 宣言ではない。
    // content property側でのgrammar相反は上記 parse_content_fn doc 参照)。
    let items = content_items("content()");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_before_keyword() {
    // §1.1.1.1 の spec 例 `h1 { string-set: header content(before) ':' content(text); }`
    // で使われる `before` keyword。
    let items = content_items("content(before)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Before,
        }]
    );
}

#[test]
fn content_content_fn_after_keyword() {
    let items = content_items("content(after)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::After,
        }]
    );
}

#[test]
fn content_content_fn_first_letter_keyword() {
    let items = content_items("content(first-letter)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::FirstLetter,
        }]
    );
}

#[test]
fn content_content_fn_rejects_unknown_keyword() {
    // GCPM 3 §1.1.1.1 の grammar は `[text | before | after | first-letter]`
    // の 4 alternative のみ (string-set/GCPM3側の文脈)。それ以外の ident は
    // parse_content_text_keyword が None を返し、上位伝播で
    // parse_content_list_items が break、declaration drop = None。`marker`
    // はこの GCPM3 grammar には無い。CSS Content 3 §2.7.3 は独自に
    // content() を `marker` 含む 5 keyword で定義しているが、この実装は
    // keyword 集合について GCPM 3 §1.1.1.1 に従うと決めており
    // (parse_content_fn の doc comment 参照)、`marker` reject は意図した
    // 挙動であって未解決の問題ではない。
    // 本 test は現状の GCPM3-scoped 実装の挙動を
    // check するものであり、`marker` が spec に一切存在しないという主張では
    // ない。
    assert_eq!(parse("content(marker)", "content"), None);
    assert_eq!(parse("content(bogus)", "content"), None);
}

#[test]
fn content_content_fn_rejects_target_text_keyword() {
    // §1.1.1.1 は `text` alternative を持つ (target-text() §2.6.3 は `content`)。
    // spec spelling divergence — `content(content)` は spec-invalid、reject。
    // 混同 (sibling ContentPart 再利用) を防ぐ regression check。
    assert_eq!(parse("content(content)", "content"), None);
}

#[test]
fn content_content_fn_case_insensitive_keyword() {
    // spec 慣行: keyword は ASCII case-insensitive。
    let items = content_items("content(TEXT)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_case_insensitive_function_name() {
    // parse_content_function は既存 arm と同じく ASCII case-insensitive dispatch。
    let items = content_items("CONTENT(before)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Before,
        }]
    );
}

#[test]
fn content_content_fn_rejects_extra_argument() {
    // grammar は single-argument。余剰 token は
    // parse_nested_block 内 parse_entirely が拒否し declaration drop。
    assert_eq!(parse("content(text, extra)", "content"), None);
    assert_eq!(parse("content(text before)", "content"), None);
}

#[test]
fn string_set_content_fn_mixed_with_other_items() {
    // §1.1.1.1 の spec 例:
    //   h1 { string-set: header content(before) ':' content(text); }
    // → (header, `[Content{Before}, Literal(":"), Content{Text}]`) 3 items。
    let entries = string_set_entries(r#"header content(before) ":" content(text)"#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("header"));
    assert_eq!(
        entries[0].1,
        vec![
            ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            },
            ContentComponent::Literal(SmolStr::new(":")),
            ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            },
        ]
    );
}

// ── page (CSS Paged Media 3 §8.1) ──

#[test]
fn page_value_parses_auto_and_custom_ident() {
    assert_eq!(
        parse("auto", "page"),
        Some(PropertyValue::Page(PageValue::Auto))
    );
    assert_eq!(
        parse("table", "page"),
        Some(PropertyValue::Page(PageValue::Named(Atom::from("table"))))
    );
    assert_eq!(
        parse("xyzabc", "page"),
        Some(PropertyValue::Page(PageValue::Named(Atom::from("xyzabc"))))
    );
    // CSS-wide keywords and `default` are not custom idents.
    for kw in [
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ] {
        assert_eq!(parse_entire(kw, "page"), None, "{kw}");
    }
    // two idents / dimension are grammar-outside (caller drops).
    assert_eq!(parse_entire("not valid", "page"), None);
    assert_eq!(parse_entire("123px", "page"), None);
}

#[test]
fn page_value_key_maps_to_page_property_key() {
    let v = PropertyValue::Page(PageValue::Auto);
    assert_eq!(v.key(), PropertyKey::Page);
}

// ── quotes property (CSS Content Module Level 3 §2.4.1 / legacy CSS2
// §12.3.1 grammar subset `none | [ <string> <string> ]+`) ──

fn quotes_pairs(pairs: &[(&str, &str)]) -> Arc<Vec<(SmolStr, SmolStr)>> {
    Arc::new(
        pairs
            .iter()
            .map(|(open, close)| (SmolStr::new(open), SmolStr::new(close)))
            .collect(),
    )
}

#[test]
fn quotes_none_returns_empty_vec() {
    // spec: `none` は空リストと同等 (top-level alternative)。
    // empty case は shared Arc slot (`empty_quotes_entries`) を使う。
    assert_eq!(
        parse("none", "quotes"),
        Some(PropertyValue::Quotes(empty_quotes_entries()))
    );
}

#[test]
fn quotes_is_case_insensitive_on_none() {
    // CSS spec: keyword `none` は ASCII case-insensitive。
    assert_eq!(
        parse("NONE", "quotes"),
        Some(PropertyValue::Quotes(empty_quotes_entries()))
    );
}

#[test]
fn quotes_single_pair() {
    assert_eq!(
        parse(r#""«" "»""#, "quotes"),
        Some(PropertyValue::Quotes(quotes_pairs(&[("«", "»")])))
    );
}

#[test]
fn quotes_multiple_pairs_deeper_nesting_levels() {
    // 2 pair 目は 1 pair 目より深い nesting level (`PropertyValue::Quotes`
    // doc の「levels of nesting」節)。
    assert_eq!(
        parse(r#""«" "»" "‹" "›""#, "quotes"),
        Some(PropertyValue::Quotes(quotes_pairs(&[
            ("«", "»"),
            ("‹", "›"),
        ])))
    );
}

#[test]
fn quotes_rejects_empty_declaration() {
    // grammar は `[ <string> <string> ]+` — 0 pair (`none` でもなく
    // 何も書かれていない) は invalid、declaration drop。
    assert_eq!(parse("", "quotes"), None);
}

#[test]
fn quotes_rejects_odd_number_of_strings() {
    // trailing unpaired <string> は `[ <string> <string> ]+` に一致しない
    // (`parse_quotes_property` doc の "malformed pair" 節)。
    assert_eq!(parse(r#""«" "»" "‹""#, "quotes"), None);
}

#[test]
fn quotes_rejects_non_string_token() {
    // <string> でない token (bare ident) は grammar 違反。
    assert_eq!(parse("open close", "quotes"), None);
}

#[test]
fn quotes_key_maps_to_quotes_property_key() {
    assert_eq!(
        PropertyValue::Quotes(empty_quotes_entries()).key(),
        PropertyKey::Quotes
    );
}
