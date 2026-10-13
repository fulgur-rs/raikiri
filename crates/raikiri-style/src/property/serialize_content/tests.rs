use super::serialize_content_value;

fn content(raw: &str) -> Option<String> {
    serialize_content_value("content", raw)
}

#[test]
fn keywords_and_plain_items_read_back_unchanged() {
    for value in [
        "none",
        "normal",
        "open-quote",
        "no-close-quote",
        "\"hello\" \"world\"",
        "url(\"picture.svg\") \"hello\"",
        "counter(counter-name, counter-style)",
        "counters(counter-name, \".\") \"potato\"",
    ] {
        assert_eq!(content(value).as_deref(), Some(value), "{value}");
    }
}

#[test]
fn the_default_decimal_counter_style_is_omitted() {
    assert_eq!(
        content("counter(counter-name, dECiMaL)").as_deref(),
        Some("counter(counter-name)")
    );
    assert_eq!(
        content("counters(counter-name, \".\", DECIMAL)").as_deref(),
        Some("counters(counter-name, \".\")")
    );
}

#[test]
fn alternative_text_follows_a_slash() {
    assert_eq!(
        content("open-quote / \"alt text\"").as_deref(),
        Some("open-quote / \"alt text\"")
    );
    assert_eq!(
        content("\"regular text\" / \"alt text 1\" counter(cnt) \"alt text 2\"").as_deref(),
        Some("\"regular text\" / \"alt text 1\" counter(cnt) \"alt text 2\"")
    );
    assert_eq!(
        content("\"\" / counter(cnt, DECIMAL)").as_deref(),
        Some("\"\" / counter(cnt)")
    );
}

#[test]
fn invalid_alternative_text_and_other_properties_are_left_alone() {
    assert_eq!(content("\"hi\" / no-close-quote"), None);
    assert_eq!(content("\"hi\" / url(\"a.svg\")"), None);
    assert_eq!(content("\"hi\" /"), None);
    assert_eq!(content("none / \"alt\""), None);
    assert_eq!(serialize_content_value("quotes", "\"a\" \"b\""), None);
}
