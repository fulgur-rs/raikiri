//! ContentValueItem canonical 10 variant construction pins と、
//! Image/Contents/Quote/Leader 4 variant construction pins
//! (design doc §7.1 canonical 10 の外、raikiri-style
//! `ContentComponent` 1:1 mirror)。
//!
//! design doc §7.1 line 1926-1937 verbatim shape (canonical 10 分)。

use super::super::*;

#[test]
fn literal_string_payload() {
    let c = ContentValueItem::Literal(String::from("hello"));
    match c {
        ContentValueItem::Literal(s) => {
            assert_eq!(s.as_str(), "hello");
        }
        other => panic!("expected Literal, got {other:?}"),
    }
}

#[test]
fn counter_default_style_is_decimal() {
    let c = ContentValueItem::Counter {
        name: Symbol::new("chapter"),
        style: CounterStyle::default(),
    };
    match c {
        ContentValueItem::Counter { name, style } => {
            assert_eq!(name.as_str(), "chapter");
            assert_eq!(style, CounterStyle::Decimal);
        }
        other => panic!("expected Counter, got {other:?}"),
    }
}

#[test]
fn counters_separator_payload() {
    let c = ContentValueItem::Counters {
        name: Symbol::new("section"),
        separator: String::from("."),
        style: CounterStyle::default(),
    };
    match c {
        ContentValueItem::Counters {
            name,
            separator,
            style,
        } => {
            assert_eq!(name.as_str(), "section");
            assert_eq!(separator, ".");
            assert_eq!(style, CounterStyle::Decimal);
        }
        other => panic!("expected Counters, got {other:?}"),
    }
}

#[test]
fn string_default_fetch_mode_is_first() {
    let c = ContentValueItem::String {
        name: Symbol::new("running-title"),
        fetch: StringFetchMode::default(),
    };
    match c {
        ContentValueItem::String { name, fetch } => {
            assert_eq!(name.as_str(), "running-title");
            assert_eq!(fetch, StringFetchMode::First);
        }
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn element_variant_construct() {
    let c = ContentValueItem::Element {
        name: Symbol::new("header"),
    };
    match c {
        ContentValueItem::Element { name } => {
            assert_eq!(name.as_str(), "header");
        }
        other => panic!("expected Element, got {other:?}"),
    }
}

#[test]
fn content_default_part_is_content() {
    let c = ContentValueItem::Content {
        part: ContentPart::default(),
    };
    match c {
        ContentValueItem::Content { part } => {
            assert_eq!(part, ContentPart::Content);
        }
        other => panic!("expected Content, got {other:?}"),
    }
}

#[test]
fn attr_variant_construct() {
    let c = ContentValueItem::Attr {
        name: Symbol::new("data-title"),
    };
    match c {
        ContentValueItem::Attr { name } => {
            assert_eq!(name.as_str(), "data-title");
        }
        other => panic!("expected Attr, got {other:?}"),
    }
}

#[test]
fn target_counter_url_payload() {
    let url = Url::parse("https://example.com/#foo").expect("valid URL");
    let c = ContentValueItem::TargetCounter {
        url: url.clone(),
        name: Symbol::new("chapter"),
        style: CounterStyle::default(),
    };
    match c {
        ContentValueItem::TargetCounter {
            url: u,
            name,
            style,
        } => {
            assert_eq!(u, url);
            assert_eq!(name.as_str(), "chapter");
            assert_eq!(style, CounterStyle::Decimal);
        }
        other => panic!("expected TargetCounter, got {other:?}"),
    }
}

#[test]
fn target_counters_sep_field_name_matches_design_doc() {
    // design doc §7.1 line 1935: `sep: String` (not `separator`)。
    // Regression check for the deliberate field-name deviation from
    // ContentComponent::TargetCounters (which uses `separator`).
    let url = Url::parse("https://example.com/#foo").expect("valid URL");
    let c = ContentValueItem::TargetCounters {
        url: url.clone(),
        name: Symbol::new("section"),
        sep: String::from("."),
        style: CounterStyle::default(),
    };
    match c {
        ContentValueItem::TargetCounters {
            url: u,
            name,
            sep,
            style,
        } => {
            assert_eq!(u, url);
            assert_eq!(name.as_str(), "section");
            assert_eq!(sep, ".");
            assert_eq!(style, CounterStyle::Decimal);
        }
        other => panic!("expected TargetCounters, got {other:?}"),
    }
}

#[test]
fn target_text_default_part_is_content() {
    let url = Url::parse("https://example.com/#foo").expect("valid URL");
    let c = ContentValueItem::TargetText {
        url: url.clone(),
        part: ContentPart::default(),
    };
    match c {
        ContentValueItem::TargetText { url: u, part } => {
            assert_eq!(u, url);
            assert_eq!(part, ContentPart::Content);
        }
        other => panic!("expected TargetText, got {other:?}"),
    }
}

#[test]
fn image_url_payload() {
    let url = Url::parse("https://example.com/logo.png").expect("valid URL");
    let c = ContentValueItem::Image { url: url.clone() };
    match c {
        ContentValueItem::Image { url: u } => {
            assert_eq!(u, url);
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => panic!("expected Image, got {other:?}"),
    }
}

#[test]
fn contents_unit_variant_construct() {
    let c = ContentValueItem::Contents;
    assert!(matches!(c, ContentValueItem::Contents));
}

#[test]
fn quote_payload_covers_all_keywords() {
    for kw in [
        QuoteKeyword::OpenQuote,
        QuoteKeyword::CloseQuote,
        QuoteKeyword::NoOpenQuote,
        QuoteKeyword::NoCloseQuote,
    ] {
        let c = ContentValueItem::Quote(kw);
        match c {
            ContentValueItem::Quote(payload) => assert_eq!(payload, kw),
            // cov:ignore: panic-message literal only executed on
            // assertion failure, which doesn't happen while this test
            // passes.
            other => panic!("expected Quote, got {other:?}"),
        }
    }
}

#[test]
fn leader_payload_covers_all_types() {
    for lt in [
        LeaderType::Dotted,
        LeaderType::Solid,
        LeaderType::Space,
        LeaderType::String(SmolStr::new("~")),
    ] {
        let c = ContentValueItem::Leader(lt.clone());
        match c {
            ContentValueItem::Leader(payload) => assert_eq!(payload, lt),
            // cov:ignore: panic-message literal only executed on
            // assertion failure, which doesn't happen while this test
            // passes.
            other => panic!("expected Leader, got {other:?}"),
        }
    }
}
