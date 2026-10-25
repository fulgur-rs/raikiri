//! Drops the internal subset of a `<!DOCTYPE>` before xml5ever tokenizes it.
//!
//! xml5ever does not read DTDs. It ends a DOCTYPE at the first `>`, which for
//! `<!DOCTYPE html [<!ENTITY a "x"> <!ENTITY b "y">]>` is the end of the first
//! declaration: the rest of the subset then reaches the prolog as comments and
//! stray text, and a start tag inside an entity value would be taken for the
//! root element. [`SkipInternalSubset`] finds the subset by the XML grammar
//! (`[` outside quotes up to the matching `]`, with comments, processing
//! instructions, and quoted literals skipped) and hands the parser the DOCTYPE
//! without it.
//!
//! The scan covers the prolog only. It stops at the end of the DOCTYPE, or
//! at the first thing that is not a comment, processing instruction, or
//! DOCTYPE, and the rest of the input passes through untouched.

use std::borrow::Cow;

use html5ever::tendril::fmt::UTF8;
use html5ever::tendril::{StrTendril, TendrilSink};

/// A parser front end that removes the internal subset of the DOCTYPE.
pub(super) struct SkipInternalSubset<P> {
    parser: P,
    /// Where the scan is; `None` once the prolog is past and everything
    /// passes through.
    state: Option<State>,
}

impl<P> SkipInternalSubset<P> {
    pub(super) fn new(parser: P) -> Self {
        Self {
            parser,
            state: Some(State::Prolog),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Between prolog items.
    Prolog,
    /// After `<` in the prolog.
    Lt,
    /// After `<!` in the prolog, matching `--` (a comment) or `DOCTYPE`.
    /// `matched` counts the characters of `target` seen; `target` is empty
    /// until the first one chooses it.
    Bang {
        target: &'static str,
        matched: usize,
    },
    /// Inside a comment; `dashes` is the run of `-` just before.
    Comment { dashes: u8 },
    /// Inside a processing instruction; `question` is set right after a `?`.
    Pi { question: bool },
    /// Inside the DOCTYPE, outside its internal subset.
    Doctype { quote: Option<char> },
    /// Inside the internal subset, which is not passed on.
    Subset(Subset),
}

/// Where the scan is inside the internal subset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Subset {
    /// In a declaration or between declarations.
    Normal,
    /// After `<`.
    Lt,
    /// After `<!`.
    Bang,
    /// After `<!-`.
    BangDash,
    Comment {
        dashes: u8,
    },
    Pi {
        question: bool,
    },
    /// Inside a quoted literal of a declaration.
    Quote(char),
}

impl State {
    /// Moves on over `c`. Returns the state after `c`, `None` once the prolog
    /// is over, and whether `c` goes to the parser.
    fn step(self, c: char) -> (Option<State>, bool) {
        match self {
            State::Prolog => (Some(if c == '<' { State::Lt } else { State::Prolog }), true),
            State::Lt => (
                match c {
                    '!' => Some(State::Bang {
                        target: "",
                        matched: 0,
                    }),
                    '?' => Some(State::Pi { question: false }),
                    _ => None,
                },
                true,
            ),
            State::Bang { target, matched } => {
                let target = if target.is_empty() {
                    match c {
                        '-' => "--",
                        'D' => "DOCTYPE",
                        _ => return (None, true),
                    }
                } else {
                    target
                };
                if target.as_bytes()[matched] as char != c {
                    return (None, true);
                }
                let matched = matched + 1;
                let next = if matched < target.len() {
                    State::Bang { target, matched }
                } else if target == "--" {
                    State::Comment { dashes: 0 }
                } else {
                    State::Doctype { quote: None }
                };
                (Some(next), true)
            }
            State::Comment { dashes } => (
                Some(match c {
                    '-' => State::Comment {
                        dashes: (dashes + 1).min(2),
                    },
                    '>' if dashes >= 2 => State::Prolog,
                    _ => State::Comment { dashes: 0 },
                }),
                true,
            ),
            State::Pi { question } => (
                Some(match c {
                    '>' if question => State::Prolog,
                    _ => State::Pi { question: c == '?' },
                }),
                true,
            ),
            State::Doctype { quote: Some(q) } => (
                Some(State::Doctype {
                    quote: (c != q).then_some(q),
                }),
                true,
            ),
            State::Doctype { quote: None } => match c {
                '"' | '\'' => (Some(State::Doctype { quote: Some(c) }), true),
                '[' => (Some(State::Subset(Subset::Normal)), false),
                '>' => (None, true),
                _ => (Some(State::Doctype { quote: None }), true),
            },
            State::Subset(subset) => match subset.step(c) {
                // The `]` that closes the subset is dropped with it.
                None => (Some(State::Doctype { quote: None }), false),
                Some(subset) => (Some(State::Subset(subset)), false),
            },
        }
    }
}

impl Subset {
    /// Moves on over `c`; `None` when `c` is the `]` that ends the subset.
    fn step(self, c: char) -> Option<Subset> {
        Some(match self {
            Subset::Normal => match c {
                '<' => Subset::Lt,
                '"' | '\'' => Subset::Quote(c),
                ']' => return None,
                _ => Subset::Normal,
            },
            Subset::Lt => match c {
                '!' => Subset::Bang,
                '?' => Subset::Pi { question: false },
                _ => return Subset::Normal.step(c),
            },
            Subset::Bang => match c {
                '-' => Subset::BangDash,
                _ => return Subset::Normal.step(c),
            },
            Subset::BangDash => match c {
                '-' => Subset::Comment { dashes: 0 },
                _ => return Subset::Normal.step(c),
            },
            Subset::Comment { dashes } => match c {
                '-' => Subset::Comment {
                    dashes: (dashes + 1).min(2),
                },
                '>' if dashes >= 2 => Subset::Normal,
                _ => Subset::Comment { dashes: 0 },
            },
            Subset::Pi { question } => match c {
                '>' if question => Subset::Normal,
                _ => Subset::Pi { question: c == '?' },
            },
            Subset::Quote(q) => {
                if c == q {
                    Subset::Normal
                } else {
                    self
                }
            }
        })
    }
}

impl<P: TendrilSink<UTF8>> TendrilSink<UTF8> for SkipInternalSubset<P> {
    type Output = P::Output;

    fn process(&mut self, t: StrTendril) {
        let Some(start) = self.state else {
            self.parser.process(t);
            return;
        };
        let text: &str = &t;
        let mut kept = String::with_capacity(text.len());
        let mut was_in_subset = matches!(start, State::Subset(_));
        let mut ignored_subset = false;
        for (index, c) in text.char_indices() {
            let Some(state) = self.state else {
                kept.push_str(&text[index..]);
                break;
            };
            let (next, forward) = state.step(c);
            let in_subset = matches!(next, Some(State::Subset(_)));
            ignored_subset |= in_subset && !was_in_subset;
            was_in_subset = in_subset;
            self.state = next;
            if forward {
                kept.push(c);
            }
        }
        if ignored_subset {
            self.parser
                .error(Cow::Borrowed("Internal DTD subset ignored"));
        }
        if !kept.is_empty() {
            self.parser.process(StrTendril::from_slice(&kept));
        }
    }

    fn error(&mut self, desc: Cow<'static, str>) {
        self.parser.error(desc);
    }

    fn finish(self) -> Self::Output {
        self.parser.finish()
    }
}

#[cfg(test)]
mod tests;
