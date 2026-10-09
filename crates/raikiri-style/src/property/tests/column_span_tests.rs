use super::*;

#[test]
fn column_span_accepts_only_none_all_and_serializes_canonical_keywords() {
    for (source, expected) in [("none", "none"), ("all", "all"), ("ALL", "all")] {
        let value = parse_entire(source, "column-span").expect("valid column-span keyword");
        assert_eq!(serialize_value(&value).as_deref(), Some(expected));
    }
    for source in ["auto", "1", "2", "0", "all none", "none all", "all 1"] {
        assert_eq!(parse_entire(source, "column-span"), None, "{source}");
    }
}
