use super::*;

#[test]
fn admission_never_exposes_a_partial_token_set() {
    for (count, admitted) in [(0, true), (512, true), (513, false)] {
        let value = (0..count)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let index = ClassIndex::new(&value, false);
        assert_eq!(index.is_some(), admitted);
        if let Some(index) = index {
            assert_eq!(index.tokens.len(), count);
            assert!(!index.contains(""));
            if count != 0 {
                assert!(index.contains(&format!("c{}", count - 1)));
            }
        }
    }
}
