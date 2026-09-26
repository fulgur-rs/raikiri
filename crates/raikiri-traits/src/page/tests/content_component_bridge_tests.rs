//! [`TryFrom<ContentComponent> for ContentValueItem`] roundtrip pins
//! (canonical taxonomy conversion; Image/Contents/Quote/Leader arms
//! added as a 1:1 mirror of raikiri-style `ContentComponent`)。
//!
//! Coverage: 13 of 14 [`ContentValueItem`] variants — [`Element`] は
//! [`ContentComponent::Element`] 未実装のため bridge 経路では現在到達
//! 不能。variant 追加時に arm を extend する。

use super::super::*;

#[test]
fn literal_bridge_converts_smol_str_to_string() {
    let cc = ContentComponent::Literal(SmolStr::new("hello"));
    let cvi = ContentValueItem::try_from(cc).expect("infallible for Literal");
    assert_eq!(cvi, ContentValueItem::Literal(String::from("hello")));
}

#[test]
fn counter_bridge_wraps_name_in_symbol() {
    let cc = ContentComponent::Counter {
        name: SmolStr::new("chapter"),
        style: CounterStyle::default(),
    };
    let cvi = ContentValueItem::try_from(cc).expect("infallible for Counter");
    assert_eq!(
        cvi,
        ContentValueItem::Counter {
            name: Symbol::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn counters_bridge_preserves_separator_field_name() {
    // ContentComponent::Counters は `separator`、ContentValueItem::Counters も
    // `separator` (design doc §7.1 line 1929 verbatim)。両者同名なので
    // straight mapping。
    let cc = ContentComponent::Counters {
        name: SmolStr::new("section"),
        separator: String::from("."),
        style: CounterStyle::default(),
    };
    let cvi = ContentValueItem::try_from(cc).expect("infallible for Counters");
    assert_eq!(
        cvi,
        ContentValueItem::Counters {
            name: Symbol::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn string_bridge_preserves_fetch_mode() {
    let cc = ContentComponent::String {
        name: SmolStr::new("title"),
        fetch: StringFetchMode::Last,
    };
    let cvi = ContentValueItem::try_from(cc).expect("infallible for String");
    assert_eq!(
        cvi,
        ContentValueItem::String {
            name: Symbol::new("title"),
            fetch: StringFetchMode::Last,
        }
    );
}

#[test]
fn attr_bridge_wraps_name_in_symbol() {
    let cc = ContentComponent::Attr {
        name: SmolStr::new("data-title"),
    };
    let cvi = ContentValueItem::try_from(cc).expect("infallible for Attr");
    assert_eq!(
        cvi,
        ContentValueItem::Attr {
            name: Symbol::new("data-title"),
        }
    );
}

#[test]
fn target_counter_bridge_parses_url() {
    let cc = ContentComponent::TargetCounter {
        url: String::from("https://example.com/#foo"),
        name: SmolStr::new("chapter"),
        style: CounterStyle::default(),
    };
    let cvi = ContentValueItem::try_from(cc).expect("valid URL");
    let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
    assert_eq!(
        cvi,
        ContentValueItem::TargetCounter {
            url: expected_url,
            name: Symbol::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn target_counters_bridge_maps_separator_to_sep() {
    // ContentComponent::TargetCounters は `separator`、design doc §7.1
    // line 1935 の ContentValueItem::TargetCounters は `sep` — bridge
    // で名称変換される。
    let cc = ContentComponent::TargetCounters {
        url: String::from("https://example.com/#foo"),
        name: SmolStr::new("section"),
        separator: String::from("."),
        style: CounterStyle::default(),
    };
    let cvi = ContentValueItem::try_from(cc).expect("valid URL");
    let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
    assert_eq!(
        cvi,
        ContentValueItem::TargetCounters {
            url: expected_url,
            name: Symbol::new("section"),
            sep: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn target_text_bridge_parses_url() {
    let cc = ContentComponent::TargetText {
        url: String::from("https://example.com/#foo"),
        part: ContentPart::Before,
    };
    let cvi = ContentValueItem::try_from(cc).expect("valid URL");
    let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
    assert_eq!(
        cvi,
        ContentValueItem::TargetText {
            url: expected_url,
            part: ContentPart::Before,
        }
    );
}

#[test]
fn content_bridge_maps_text_keyword_to_content_part() {
    // ContentTextKeyword::Text (GCPM 3 §1.1.1.1) →
    // ContentPart::Content (CSS Content 3 §2.6.3): 両者とも「要素の
    // string value 全体」を指す同一概念への canonical mapping
    // (spec の "default" 宣言には依らない — 詳細は
    // content_text_keyword_to_content_part の doc comment 参照)。
    let cc = ContentComponent::Content {
        keyword: ContentTextKeyword::Text,
    };
    let cvi = ContentValueItem::try_from(cc).expect("infallible for Content");
    assert_eq!(
        cvi,
        ContentValueItem::Content {
            part: ContentPart::Content,
        }
    );
}

#[test]
fn content_bridge_maps_before_after_first_letter_verbatim() {
    // 非-default keyword は同名の ContentPart 値に mapping。
    for (kw, expected) in [
        (ContentTextKeyword::Before, ContentPart::Before),
        (ContentTextKeyword::After, ContentPart::After),
        (ContentTextKeyword::FirstLetter, ContentPart::FirstLetter),
    ] {
        let cvi = ContentValueItem::try_from(ContentComponent::Content { keyword: kw })
            .expect("infallible for Content");
        assert_eq!(
            cvi,
            ContentValueItem::Content { part: expected },
            "keyword {kw:?} should map to part {expected:?}"
        );
    }
}

#[test]
fn image_bridge_parses_url() {
    let cc = ContentComponent::Image {
        url: String::from("https://example.com/logo.png"),
    };
    let cvi = ContentValueItem::try_from(cc).expect("valid URL");
    let expected_url = Url::parse("https://example.com/logo.png").expect("URL literal parses");
    assert_eq!(cvi, ContentValueItem::Image { url: expected_url });
}

#[test]
fn contents_bridge_maps_to_unit_variant() {
    let cvi =
        ContentValueItem::try_from(ContentComponent::Contents).expect("infallible for Contents");
    assert_eq!(cvi, ContentValueItem::Contents);
}

#[test]
fn quote_bridge_passes_keyword_through() {
    for kw in [
        QuoteKeyword::OpenQuote,
        QuoteKeyword::CloseQuote,
        QuoteKeyword::NoOpenQuote,
        QuoteKeyword::NoCloseQuote,
    ] {
        let cvi =
            ContentValueItem::try_from(ContentComponent::Quote(kw)).expect("infallible for Quote");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(cvi, ContentValueItem::Quote(kw), "keyword {kw:?} roundtrip");
    }
}

#[test]
fn leader_bridge_passes_type_through() {
    for lt in [
        LeaderType::Dotted,
        LeaderType::Solid,
        LeaderType::Space,
        LeaderType::String(SmolStr::new("~")),
    ] {
        let cvi = ContentValueItem::try_from(ContentComponent::Leader(lt.clone()))
            .expect("infallible for Leader");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            cvi,
            ContentValueItem::Leader(lt.clone()),
            "leader type {lt:?} roundtrip"
        );
    }
}

#[test]
fn image_bridge_returns_invalid_url_error() {
    // Invalid URL (relative URL に base 無し) は InvalidUrl error
    // (sibling target_counter_bridge_returns_invalid_url_error と同じ pattern)。
    let cc = ContentComponent::Image {
        url: String::from("not a url"),
    };
    let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
    assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
}

#[test]
fn target_counter_bridge_returns_invalid_url_error() {
    // Invalid URL (relative URL に base 無し) は InvalidUrl error。
    let cc = ContentComponent::TargetCounter {
        url: String::from("not a url"),
        name: SmolStr::new("chapter"),
        style: CounterStyle::default(),
    };
    let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
    assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
}

#[test]
fn target_counters_bridge_propagates_url_parse_error() {
    let cc = ContentComponent::TargetCounters {
        url: String::from("not a url"),
        name: SmolStr::new("section"),
        separator: String::from("."),
        style: CounterStyle::default(),
    };
    let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
    assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
}

#[test]
fn target_text_bridge_propagates_url_parse_error() {
    let cc = ContentComponent::TargetText {
        url: String::from("not a url"),
        part: ContentPart::default(),
    };
    let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
    assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
}

#[test]
fn convert_error_display_and_source_chain() {
    use std::error::Error as _;
    // InvalidUrl は source() で url::ParseError を surface。
    let parse_err = Url::parse("not a url").expect_err("invalid URL");
    let err = ContentValueConvertError::InvalidUrl(parse_err);
    assert!(err.to_string().contains("invalid URL"));
    assert!(err.source().is_some());

    // UnsupportedVariant は source() none、Display で origin cite。
    let uv = ContentValueConvertError::UnsupportedVariant;
    assert!(uv.to_string().contains("unsupported ContentComponent"));
    assert!(uv.source().is_none());
}
