//! Whether the first letter of the text gathered so far is complete.

use icu_properties::{CodePointMapData, props::GraphemeClusterBreak as G};

/// What the text gathered so far is known to hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gathered {
    /// Nothing is known: all of the text is scanned.
    Unknown,
    /// Only white space, which comes before the first letter.
    Blank,
    /// White space, then punctuation (and the spaces between) that goes before
    /// the first letter. The letter may still follow.
    Punctuation,
    /// A letter that ends the text, with the punctuation that follows it, if
    /// any. More punctuation may still follow.
    Letter,
    /// The search ended without a letter, and no text that follows changes it.
    Closed,
}

/// The first-letter search over text that grows one piece at a time.
///
/// The scan of [`shodo::first_letter_range`] starts at the beginning of the
/// text, so asking it again after every piece costs the whole text each time.
/// A long run of pieces that hold only punctuation, before the letter or after
/// it, would cost the square of its length. The text is known to end in
/// punctuation of the kind that the next piece continues, and the piece is
/// checked by itself.
pub(super) struct LetterScan {
    gathered: Gathered,
    /// The text that a piece is checked in, kept to avoid allocating for each
    /// piece.
    probe: String,
}

/// The text that ends every probe. It is a control character, so it is a
/// cluster of its own and is the letter when nothing before it is.
const END: char = '\0';

/// What stands for the letter and the punctuation after it, in front of a piece
/// that is checked by itself, so that the piece joins it as it would join the
/// text: a letter. Like the last cluster of the text it stands for, it only has
/// the characters that extend a cluster join it.
const AFTER_LETTER: &str = "A";

/// What stands for the text before a piece that is checked by itself when there
/// is no letter yet: nothing for white space, and an opening bracket for
/// punctuation before the letter. The spaces between punctuation marks join
/// the bracket, as they join a mark that came before them.
fn before_letter(blank: bool) -> &'static str {
    if blank { "" } else { "(" }
}

impl LetterScan {
    pub(super) fn new() -> Self {
        Self {
            gathered: Gathered::Unknown,
            probe: String::new(),
        }
    }

    /// Whether the first letter ends before the end of `joined`, the text
    /// gathered so far, when `joined[start..]` is the piece that was added.
    pub(super) fn is_complete(&mut self, joined: &str, start: usize) -> bool {
        match self.gathered {
            Gathered::Closed => false,
            Gathered::Blank | Gathered::Punctuation | Gathered::Letter => {
                if self.extends_cluster(joined, start) || self.absorbs(&joined[start..]) {
                    false
                } else {
                    self.ask(joined)
                }
            }
            Gathered::Unknown => self.ask(joined),
        }
    }

    /// Whether the piece only extends the last cluster of a known letter.
    fn extends_cluster(&self, joined: &str, start: usize) -> bool {
        if self.gathered != Gathered::Letter {
            return false;
        }
        let categories = CodePointMapData::<G>::new();
        // GB4/5 takes precedence over GB9/9a: even extenders start a new
        // cluster after a control. Keep that case with the full scan.
        if joined[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| matches!(categories.get(ch), G::Control | G::CR | G::LF))
        {
            return false;
        }
        // Match the ICU grapheme categories used by shodo 0.0.39. The
        // differential tests guard this shortcut against its full scan.
        joined[start..].chars().all(|ch| {
            #[cfg(test)]
            super::tests::SCANNED_BYTES.with(|count| count.set(count.get() + ch.len_utf8()));
            matches!(categories.get(ch), G::Extend | G::ZWJ | G::SpacingMark)
        })
    }

    /// Whether `piece` only adds to the text what the text already ends in,
    /// and keeps what is known of it: white space and punctuation before the
    /// letter that there is none of yet, or punctuation after the letter. Text
    /// that does is not complete, as it has nothing after the letter.
    fn absorbs(&mut self, piece: &str) -> bool {
        if self.gathered == Gathered::Letter {
            // Whatever the piece is, it takes the letter and the punctuation
            // after it as far as the end of the piece, so it joins no
            // different kind of cluster than the letter does. Punctuation that
            // goes before a letter, a mark that extends a cluster and the like
            // stay with the scan of all of the text.
            return self
                .probe_range("", piece)
                .is_some_and(|range| range.end == piece.len() + END.len_utf8())
                && self.probe_range(AFTER_LETTER, piece).is_some_and(|range| {
                    range.start == 0 && range.end == AFTER_LETTER.len() + piece.len()
                });
        }
        let blank = self.gathered == Gathered::Blank;
        let Some(range) = self.probe_range(before_letter(blank), piece) else {
            return false;
        };
        // The letter is the end of the probe, so the piece is white space and
        // punctuation before it. The scan starts where the first of them is:
        // before the piece when the text is not blank, after white space in it
        // otherwise.
        if range.end != self.probe.len() {
            return false;
        }
        self.gathered = if blank && range.start == piece.len() {
            Gathered::Blank
        } else {
            Gathered::Punctuation
        };
        true
    }

    /// The first letter of `lead`, `piece` and the end of the probe.
    fn probe_range(&mut self, lead: &str, piece: &str) -> Option<std::ops::Range<usize>> {
        self.probe.clear();
        self.probe.push_str(lead);
        self.probe.push_str(piece);
        self.probe.push(END);
        #[cfg(test)]
        super::tests::SCANNED_BYTES.with(|count| count.set(count.get() + self.probe.len()));
        shodo::first_letter_range(&self.probe, true)
    }

    /// Scan all of `joined`.
    fn ask(&mut self, joined: &str) -> bool {
        #[cfg(test)]
        super::tests::SCANNED_BYTES.with(|count| count.set(count.get() + joined.len()));
        match shodo::first_letter_range(joined, true) {
            Some(range) => {
                self.gathered = if range.end == joined.len() {
                    Gathered::Letter
                } else {
                    Gathered::Unknown
                };
                range.end < joined.len()
            }
            None => {
                self.gathered = self.classify(joined);
                false
            }
        }
    }

    /// What text is when it has no first letter.
    fn classify(&mut self, joined: &str) -> Gathered {
        match self.probe_range(joined, "") {
            Some(range) if range.end == self.probe.len() => {
                if range.start == joined.len() {
                    Gathered::Blank
                } else {
                    Gathered::Punctuation
                }
            }
            _ => Gathered::Closed,
        }
    }
}
