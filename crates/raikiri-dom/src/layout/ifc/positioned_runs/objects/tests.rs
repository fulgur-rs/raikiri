use super::*;

#[test]
fn prefix_state_matches_the_original_stack_for_nested_and_unmatched_controls() {
    let alphabet = [
        'x', '\u{202A}', '\u{202B}', '\u{202D}', '\u{202E}', '\u{202C}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}',
    ];
    let mut seed = 17_u64;
    for _ in 0..128 {
        let mut text = String::new();
        for _ in 0..128 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            text.push(alphabet[(seed >> 32) as usize % alphabet.len()]);
        }
        let actual = prefix_states(&text);
        let mut open = Vec::new();
        for (offset, c) in text.char_indices() {
            assert_eq!(actual[offset], open.is_empty());
            match c {
                '\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}' => open.push(false),
                '\u{2066}'..='\u{2068}' => open.push(true),
                '\u{202C}' if open.last() == Some(&false) => {
                    open.pop();
                }
                '\u{2069}' => {
                    if let Some(index) = open.iter().rposition(|&v| v) {
                        open.truncate(index);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(actual[text.len()], open.is_empty());
    }
}

#[test]
fn coverage_unions_overlaps_and_clips_ranges_without_losing_literal_slots() {
    let text = "a\u{FFFC}b\u{FFFC}c\u{FFFC}";
    assert_eq!(uncovered_slots(text, 10..22, Vec::new()), [11, 15, 19]);
    assert_eq!(
        uncovered_slots(text, 10..22, vec![8..12, 11..17, 15..18, 25..30, 20..20]),
        [19]
    );
    assert!(uncovered_slots(text, 10..22, std::iter::once(0..100).collect()).is_empty());
}

#[test]
fn unmatched_pdi_under_many_embeddings_is_processed_once_per_character() {
    for count in [32, 64, 128] {
        let text = format!(
            "{}{}{}",
            "\u{202A}".repeat(count),
            "\u{2069}".repeat(count),
            "\u{202C}".repeat(count)
        );
        let before = super::super::tests::image_offset_tests::prefix_visits();
        let closed = prefix_states(&text);
        assert_eq!(
            super::super::tests::image_offset_tests::prefix_visits() - before,
            3 * count
        );
        assert!(!closed[6 * count]);
        assert!(closed[text.len()]);
    }
}
