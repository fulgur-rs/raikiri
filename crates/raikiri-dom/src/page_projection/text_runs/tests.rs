use super::*;

#[test]
fn each_glyph_covers_its_cluster_up_to_the_next_one() {
    // Clusters at bytes 10, 11 and 13 of a run spanning 10..15; the glyph
    // of cluster 11 is a ligature of two characters.
    assert_eq!(
        glyph_text_ranges(&[10, 11, 13], 10, 15),
        vec![0..1, 1..3, 3..5]
    );
}

#[test]
fn glyphs_of_one_cluster_share_its_range() {
    // A base and its mark share cluster 4.
    assert_eq!(glyph_text_ranges(&[4, 4, 6], 4, 8), vec![0..2, 0..2, 2..4]);
}

#[test]
fn a_cluster_outside_the_run_is_clamped_to_the_run_text() {
    // A cluster shared with the previous run starts before this run.
    assert_eq!(glyph_text_ranges(&[2, 5], 3, 7), vec![0..2, 2..4]);
    assert_eq!(glyph_text_ranges(&[9], 3, 7), vec![4..4]);
}

#[test]
fn variations_keep_their_axis_and_value() {
    let variation = shodo::style::FontVariation {
        tag: *b"wght",
        value: 650.0,
    };
    assert_eq!(
        font_variation(&variation),
        FontVariation {
            tag: Tag(*b"wght"),
            value: 650.0,
        }
    );
}

#[test]
fn ellipsis_text_is_one_ellipsis_or_one_period_per_glyph() {
    assert_eq!(
        ellipsis_text(1, false),
        ("\u{2026}", std::iter::once(0..3).collect())
    );
    assert_eq!(ellipsis_text(3, false), ("...", vec![0..1, 1..2, 2..3]));
    // A period of a fallback split across fonts is a run of its own.
    assert_eq!(
        ellipsis_text(1, true),
        (".", std::iter::once(0..1).collect())
    );
    assert_eq!(ellipsis_text(2, true), ("..", vec![0..1, 1..2]));
    // Extra glyphs share the last period.
    assert_eq!(
        ellipsis_text(4, false),
        ("...", vec![0..1, 1..2, 2..3, 2..3])
    );
}
