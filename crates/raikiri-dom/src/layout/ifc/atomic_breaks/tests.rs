use super::*;
use SoftBreakOpportunity::{Allowed, Emergency, Hyphen, Prohibited};

fn decide(text: &str, offset: usize, standard: SoftBreakOpportunity) -> LineBreakOverride {
    around_atomics(LineBreakContext {
        text,
        offset,
        standard,
    })
}

/// The decision after an atomic inline that Unicode would not break before
/// `c`.
fn after_atomic(c: char) -> LineBreakOverride {
    decide(&format!("{OBJECT}{c}"), OBJECT.len_utf8(), Prohibited)
}

/// The decision before an atomic inline that Unicode would not break after
/// `c`.
fn before_atomic(c: char) -> LineBreakOverride {
    decide(&format!("{c}{OBJECT}"), c.len_utf8(), Prohibited)
}

#[test]
fn a_no_break_space_next_to_an_atomic_does_not_keep_it() {
    assert_eq!(after_atomic('\u{a0}'), LineBreakOverride::Allow);
    assert_eq!(before_atomic('\u{a0}'), LineBreakOverride::Allow);
}

#[test]
fn punctuation_next_to_an_atomic_does_not_keep_it() {
    // One character of each class that Unicode keeps with the object on one
    // side or the other: IS, CL, CP, EX, SY, OP, QU, HY, IN and NS.
    for c in [
        ':', '、', ')', '!', '/', '(', '"', '-', '\u{2026}', '\u{3005}',
    ] {
        assert_eq!(after_atomic(c), LineBreakOverride::Allow, "after, {c:?}");
        assert_eq!(before_atomic(c), LineBreakOverride::Allow, "before, {c:?}");
    }
}

#[test]
fn glue_and_joiners_keep_an_atomic_with_them() {
    // GL: narrow no-break space, figure space, non-breaking hyphen. WJ: word
    // joiner, zero width no-break space. ZWJ.
    for c in [
        '\u{202f}', '\u{2007}', '\u{2011}', '\u{2060}', '\u{feff}', '\u{200d}',
    ] {
        assert_eq!(
            after_atomic(c),
            LineBreakOverride::UseStandard,
            "after, {c:?}"
        );
        assert_eq!(
            before_atomic(c),
            LineBreakOverride::UseStandard,
            "before, {c:?}"
        );
    }
}

#[test]
fn white_space_next_to_an_atomic_is_left_to_the_white_space_rules() {
    for c in [' ', '\u{200b}'] {
        assert_eq!(
            after_atomic(c),
            LineBreakOverride::UseStandard,
            "after, {c:?}"
        );
        assert_eq!(
            before_atomic(c),
            LineBreakOverride::UseStandard,
            "before, {c:?}"
        );
    }
}

#[test]
fn an_opportunity_unicode_already_has_is_left_alone() {
    let text = format!("{OBJECT}\u{a0}");
    for standard in [Allowed, Emergency, Hyphen] {
        assert_eq!(
            decide(&text, OBJECT.len_utf8(), standard),
            LineBreakOverride::UseStandard,
            "{standard:?}"
        );
    }
}

#[test]
fn a_boundary_with_no_atomic_beside_it_is_left_alone() {
    assert_eq!(decide("a:", 1, Prohibited), LineBreakOverride::UseStandard);
    assert_eq!(
        decide("a\u{a0}b", 1, Prohibited),
        LineBreakOverride::UseStandard
    );
    // The ends of the text have only one side.
    let text = format!("{OBJECT}");
    assert_eq!(decide(&text, 0, Prohibited), LineBreakOverride::UseStandard);
    assert_eq!(
        decide(&text, OBJECT.len_utf8(), Prohibited),
        LineBreakOverride::UseStandard
    );
}

#[test]
fn a_combining_mark_keeps_an_atomic_with_it() {
    // U+034F COMBINING GRAPHEME JOINER is in the list of glue characters of
    // the specification, and it has the class of a combining mark.
    assert_eq!(before_atomic('\u{34f}'), LineBreakOverride::UseStandard);
    assert_eq!(after_atomic('\u{34f}'), LineBreakOverride::UseStandard);
}

#[test]
fn bidi_controls_between_an_atomic_and_its_neighbour_are_looked_through() {
    // An isolate around the colon: the cut stays before the controls.
    let after = format!("{OBJECT}\u{2067}:\u{2069}");
    for offset in [OBJECT.len_utf8(), OBJECT.len_utf8() + '\u{2067}'.len_utf8()] {
        assert_eq!(
            decide(&after, offset, Prohibited),
            LineBreakOverride::Allow,
            "after, {offset}"
        );
    }
    let before = format!(":\u{2069}{OBJECT}");
    for offset in [1, 1 + '\u{2069}'.len_utf8()] {
        assert_eq!(
            decide(&before, offset, Prohibited),
            LineBreakOverride::Allow,
            "before, {offset}"
        );
    }
    // The characters beyond the controls still decide.
    let glue = format!("{OBJECT}\u{2067}\u{202f}\u{2069}");
    assert_eq!(
        decide(&glue, OBJECT.len_utf8(), Prohibited),
        LineBreakOverride::UseStandard
    );
    // Controls with nothing but the controls beyond them are no neighbour.
    let bare = format!("{OBJECT}\u{2069}");
    assert_eq!(
        decide(&bare, OBJECT.len_utf8(), Prohibited),
        LineBreakOverride::UseStandard
    );
}

#[test]
fn a_forced_break_next_to_an_atomic_is_left_to_the_standard_rules() {
    // BK, CR, LF and NL: Unicode never breaks before them, and it breaks
    // after them whatever is next.
    for c in ['\u{b}', '\r', '\n', '\u{85}', '\u{2028}', '\u{2029}'] {
        assert_eq!(
            after_atomic(c),
            LineBreakOverride::UseStandard,
            "after, {c:?}"
        );
        assert_eq!(
            before_atomic(c),
            LineBreakOverride::UseStandard,
            "before, {c:?}"
        );
    }
}
