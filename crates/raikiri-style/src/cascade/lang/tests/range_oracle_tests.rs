use super::super::{DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS, language_range_matches};

/// A reference for `language_range_matches` that reads the algorithm as
/// plainly as possible: both strings are split into vectors of owned
/// subtags before anything is compared, and the matching loop indexes them.
/// The table of deprecated subtags is the production one: it is data, and
/// the rules around it are what this reference pins.
fn range_matches_by_vectors(range: &str, content_language: &str) -> bool {
    fn subtags_match(range_subtag: &str, tag_subtag: &str) -> bool {
        range_subtag == "*" || range_subtag.eq_ignore_ascii_case(tag_subtag)
    }
    fn is_singleton(subtag: &str) -> bool {
        subtag.chars().count() == 1
    }
    fn alphanum_subtag(subtag: &str) -> bool {
        (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
    }
    fn well_formed_tag(subtags: &[&str]) -> bool {
        let first = subtags[0];
        let first_ok = if first.eq_ignore_ascii_case("x") {
            subtags.len() >= 2
        } else {
            (2..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic())
        };
        first_ok && subtags[1..].iter().all(|s| alphanum_subtag(s))
    }
    fn well_formed_range(subtags: &[&str]) -> bool {
        let first = subtags[0];
        let first_ok = first == "*"
            || ((1..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic()));
        first_ok && subtags[1..].iter().all(|&s| s == "*" || alphanum_subtag(s))
    }
    fn canonicalize(subtags: &mut [String]) {
        for &(deprecated, preferred) in DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS {
            if subtags[0].eq_ignore_ascii_case(deprecated) {
                subtags[0] = preferred.to_string();
                break;
            }
        }
    }

    if range.is_empty() || content_language.is_empty() {
        return range == content_language;
    }
    let range_subtags: Vec<&str> = range.split('-').collect();
    let tag_subtags: Vec<&str> = content_language.split('-').collect();
    if !well_formed_range(&range_subtags) || !well_formed_tag(&tag_subtags) {
        return false;
    }
    let mut range_subtags: Vec<String> = range_subtags.into_iter().map(String::from).collect();
    let mut tag_subtags: Vec<String> = tag_subtags.into_iter().map(String::from).collect();
    canonicalize(&mut range_subtags);
    canonicalize(&mut tag_subtags);
    if !subtags_match(&range_subtags[0], &tag_subtags[0]) {
        return false;
    }
    let mut ri = 1;
    let mut ti = 1;
    while ri < range_subtags.len() {
        let r = &range_subtags[ri];
        if r == "*" {
            ri += 1;
            continue;
        }
        let Some(t) = tag_subtags.get(ti) else {
            return false;
        };
        if subtags_match(r, t) {
            ri += 1;
            ti += 1;
            continue;
        }
        if is_singleton(t) {
            return false;
        }
        ti += 1;
    }
    true
}

#[test]
fn ranges_match_as_they_did_when_every_subtag_was_copied() {
    // Subtags that reach each rule: the wildcard, singletons, the private
    // use `x`, deprecated primaries in both letter cases and their
    // preferred values, scripts, regions, an overlong subtag, digits, an
    // empty subtag (a doubled hyphen) and non-ASCII text.
    const SUBTAGS: [&str; 22] = [
        "en",
        "EN",
        "us",
        "US",
        "*",
        "x",
        "X",
        "a",
        "iw",
        "IW",
        "he",
        "in",
        "id",
        "Latn",
        "1234",
        "foo",
        "abcdefghi",
        "",
        "ja",
        "mo",
        "ro",
        "é",
    ];
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let random_tag = |next: &mut dyn FnMut(usize) -> usize| -> String {
        // Sometimes the empty string, which is not a tag at all.
        if next(40) == 0 {
            return String::new();
        }
        let count = 1 + next(5);
        (0..count)
            .map(|_| SUBTAGS[next(SUBTAGS.len())])
            .collect::<Vec<_>>()
            .join("-")
    };
    let (mut matched, mut unmatched) = (0, 0);
    for _ in 0..60_000 {
        let tag = random_tag(&mut next);
        // Half of the ranges come from the tag: a prefix of it with some
        // subtags replaced, so that many of them match.
        let range = if next(2) == 0 {
            tag.split('-')
                .take(1 + next(4))
                .map(|subtag| match next(6) {
                    0 => "*",
                    1 => SUBTAGS[next(SUBTAGS.len())],
                    _ => subtag,
                })
                .collect::<Vec<_>>()
                .join("-")
        } else {
            random_tag(&mut next)
        };
        let expected = range_matches_by_vectors(&range, &tag);
        assert_eq!(
            language_range_matches(&range, &tag),
            expected,
            "{range:?} against {tag:?}"
        );
        if expected {
            matched += 1;
        } else {
            unmatched += 1;
        }
    }
    // The cases reach both outcomes, in numbers well below what the seed
    // gives now (about 11,900 and 48,100), so that a change to the
    // generator that stops reaching one of them fails here rather than
    // leaving the comparison vacuous.
    assert!(
        matched > 5_000 && unmatched > 20_000,
        "matched {matched}, unmatched {unmatched}"
    );
}
