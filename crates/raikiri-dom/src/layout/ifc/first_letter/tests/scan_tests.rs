//! The first-letter search over text that grows one piece at a time.

use super::*;
use scan::LetterScan;

/// What the search says after each piece when all of the text is scanned each
/// time.
fn scanned_every_time(pieces: &[&str]) -> Vec<bool> {
    let mut joined = String::new();
    pieces
        .iter()
        .map(|piece| {
            joined.push_str(piece);
            shodo::first_letter_range(&joined, true).is_some_and(|range| range.end < joined.len())
        })
        .collect()
}

/// What the search says after each piece through [`LetterScan`].
fn scanned_by_piece(pieces: &[&str]) -> Vec<bool> {
    let mut joined = String::new();
    let mut scan = LetterScan::new();
    pieces
        .iter()
        .map(|piece| {
            let start = joined.len();
            joined.push_str(piece);
            scan.is_complete(&joined, start)
        })
        .collect()
}

/// Pieces that tell the kinds of text apart: white space of each kind,
/// punctuation that goes before and after a letter, letters, characters that
/// join the cluster before them or after them, and line breaks.
const PIECES: &[&str] = &[
    "(",
    ")",
    "\"",
    "-",
    ".",
    " ",
    "  ",
    "\t",
    "\n",
    "\r",
    "\r\n",
    "\u{a0}",
    "\u{2002}",
    "\u{3000}",
    "\u{1361}",
    "\u{201C}",
    "\u{300C}",
    "\u{3002}",
    "\u{203C}",
    "\u{200D}",
    "\u{301}",
    "\u{903}",
    "\u{200b}",
    "\u{fe0f}",
    "\u{1f3fb}",
    "\u{600}",
    "\u{1F469}",
    "\u{1F4BB}",
    "\u{1F1E6}",
    "\u{1100}",
    "\u{0}",
    "$",
    "A",
    "\u{e9}",
    "1",
    "\u{65e5}",
    "A.",
    ".A",
    "!",
    "?",
    ";",
    "\u{ff09}",
    "\u{2002}.",
    "\u{1F469}\u{200D}",
    ")\u{301}",
    ". ",
    "\u{a0}.",
    "(\u{301}",
    " (",
    "( ",
];

/// A number sequence that is the same on every run.
struct Sequence(u64);

impl Sequence {
    fn next(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % below as u64) as usize
    }
}

#[test]
fn the_search_by_piece_says_what_the_search_of_all_the_text_says() {
    let mut random = Sequence(0x9E37_79B9_7F4A_7C15);
    for round in 0..40_000 {
        let length = 1 + random.next(14);
        let pieces: Vec<&str> = (0..length)
            .map(|_| PIECES[random.next(PIECES.len())])
            .collect();
        assert_eq!(
            scanned_by_piece(&pieces),
            scanned_every_time(&pieces),
            "round {round}: {pieces:?}"
        );
    }
}

#[test]
fn the_search_by_piece_says_what_the_search_of_all_the_text_says_for_runs_of_punctuation() {
    // A long run of pieces of punctuation, as one piece each, before the letter
    // and after it, with whatever may follow the run.
    let runs = [
        "(",
        "(\u{301}",
        " ",
        "\u{201C}",
        "\u{300C}",
        "( ",
        "\u{203C}",
        "A.",
        "\u{201C}A",
        "(A",
        ".",
        ")",
        "\u{3002}",
        "\u{2002}.",
        "\u{1F469}\u{200D}",
        "\u{600}",
        "\u{0}",
        "A",
    ];
    for run in runs {
        for tail in PIECES {
            for after in ["A", ".", " ", "\u{301}", "\u{1F4BB}", "\n", ")", "\u{3002}"] {
                let mut pieces = vec![run; 6];
                pieces.push(tail);
                pieces.push(after);
                pieces.push(tail);
                assert_eq!(
                    scanned_by_piece(&pieces),
                    scanned_every_time(&pieces),
                    "{pieces:?}"
                );
            }
        }
    }
}

#[test]
fn a_run_of_pieces_of_punctuation_is_not_scanned_over_and_over() {
    // Each piece was scanned with all of the text before it, so the bytes
    // scanned grew with the square of the number of pieces.
    let scanned = |count: usize| {
        SCANNED_BYTES.with(|bytes| bytes.set(0));
        let mut joined = String::new();
        let mut scan = LetterScan::new();
        for _ in 0..count {
            let start = joined.len();
            joined.push('(');
            assert!(!scan.is_complete(&joined, start));
        }
        let start = joined.len();
        joined.push_str("Ab");
        assert!(scan.is_complete(&joined, start));
        SCANNED_BYTES.with(|bytes| bytes.get())
    };
    let few = scanned(256);
    let many = scanned(1024);
    assert!(few < 256 * 8, "{few} bytes for 256 pieces");
    assert!(many < 1024 * 8, "{many} bytes for 1024 pieces");
}

#[test]
fn a_run_of_pieces_of_punctuation_after_the_letter_is_not_scanned_over_and_over() {
    let scanned = |count: usize| {
        SCANNED_BYTES.with(|bytes| bytes.set(0));
        let mut joined = String::from("A");
        let mut scan = LetterScan::new();
        assert!(!scan.is_complete(&joined, 0));
        for _ in 0..count {
            let start = joined.len();
            joined.push('.');
            assert!(!scan.is_complete(&joined, start));
        }
        let start = joined.len();
        joined.push('b');
        assert!(scan.is_complete(&joined, start));
        SCANNED_BYTES.with(|bytes| bytes.get())
    };
    let few = scanned(256);
    let many = scanned(1024);
    assert!(few < 256 * 8, "{few} bytes for 256 pieces");
    assert!(many < 1024 * 8, "{many} bytes for 1024 pieces");
}

#[test]
fn a_letter_that_follows_many_pieces_of_punctuation_takes_them_all() {
    let count = 300;
    let fixture = sheet_fixture("div::first-letter{font-size:20px}", "", |doc, root| {
        for _ in 0..count {
            doc.append_text(root, "(");
        }
        doc.append_text(root, "Ab");
    });
    let children = fixture.doc.get_node(fixture.root).unwrap().children.clone();
    let fonts = ahem_fonts();
    let limits = shodo::limits::Limits::default();
    let mut builder = paragraph_builder(&fixture, &fonts, &limits);
    let mut letter = FirstLetter::new(&fixture.doc, &fixture.cascade, fixture.root, &limits);
    let cv = &fixture.cascade.computed[fixture.root];
    SCANNED_BYTES.with(|bytes| bytes.set(0));
    CONTAINER_LOOKUPS.with(|lookups| lookups.set(0));
    for &child in &children {
        let text = fixture.doc.get_node(child).unwrap().text_content().unwrap();
        letter
            .push(
                &mut builder,
                &fixture.doc,
                &fixture.cascade,
                TextSource::Dom {
                    node: NodeId(child as u64),
                    offset: 0,
                },
                cv,
                text,
                &fonts,
            )
            .unwrap();
    }
    let scanned = SCANNED_BYTES.with(|bytes| bytes.get());
    assert!(scanned < count * 16, "{scanned} bytes for {count} pieces");
    // The texts still to take their part of the letter are not gone over
    // again for each text that takes its part.
    let lookups = CONTAINER_LOOKUPS.with(|lookups| lookups.get());
    assert!(lookups < count * 12, "{lookups} lookups for {count} pieces");
    // Every piece of punctuation and the letter are styled, and the rest of
    // the last text is not.
    assert_eq!(letter.styles.len(), count + 1);
    let last = letter.styles.last().unwrap();
    assert_eq!(last.source_owner, children[count]);
    assert_eq!(last.source_range, Some(0..1));
    let mut cx = shodo::LayoutContext::new();
    let paragraph = builder.build(&mut cx, &fonts).unwrap();
    assert_eq!(paragraph.text().len(), count + 2);
}

#[test]
fn extender_pieces_after_a_letter_take_linear_work() {
    for count in [256, 1024, 4096] {
        SCANNED_BYTES.with(|bytes| bytes.set(0));
        let mut joined = String::from("e");
        let mut scan = LetterScan::new();
        assert!(!scan.is_complete(&joined, 0));
        for _ in 0..count {
            let start = joined.len();
            joined.push('\u{301}');
            assert!(!scan.is_complete(&joined, start));
        }
        let start = joined.len();
        joined.push_str("Ab");
        assert!(scan.is_complete(&joined, start));
        let scanned = SCANNED_BYTES.with(|bytes| bytes.get());
        assert!(scanned < count * 16, "{scanned} bytes for {count} pieces");
    }
}

#[test]
fn extender_runs_preserve_cluster_boundaries() {
    for head in PIECES {
        for extender in ["\u{301}", "\u{903}", "\u{200d}", "\u{fe0f}", "\u{1f3fb}"] {
            for tail in PIECES {
                let mut pieces = vec![*head];
                pieces.extend([extender; 8]);
                pieces.push(tail);
                pieces.push("Ab");
                assert_eq!(
                    scanned_by_piece(&pieces),
                    scanned_every_time(&pieces),
                    "{pieces:?}"
                );
            }
        }
    }
}
