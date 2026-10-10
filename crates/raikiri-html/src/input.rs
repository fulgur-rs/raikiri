//! Incremental UTF-8 input for the html5ever parsers.
//!
//! [`Utf8Feed`] sits in front of an html5ever parser and accepts the document
//! as a sequence of byte chunks. Each chunk is validated as UTF-8 and handed
//! to the tokenizer straight away, so the parser never needs the whole input
//! in one buffer. A character split across two chunks is carried over (at
//! most three bytes) until the next chunk completes it.
//!
//! html5ever's own `from_utf8()` adapter is not used because it decodes
//! lossily (invalid sequences become U+FFFD); this crate reports invalid
//! input as [`ParseError::Encoding`] instead.

use std::io::{ErrorKind, Read};

use html5ever::tendril::{StrTendril, TendrilSink};
use raikiri_traits::ParseError;

/// Bytes requested from a reader per read call.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Push-style UTF-8 front end for an html5ever parser.
pub(crate) struct Utf8Feed<P> {
    parser: P,
    /// Leading bytes of a character whose remaining bytes have not arrived.
    pending: [u8; 4],
    pending_len: usize,
    /// Bytes already handed to the parser, used for error offsets.
    consumed: usize,
}

impl<P: TendrilSink<html5ever::tendril::fmt::UTF8>> Utf8Feed<P> {
    pub(crate) fn new(parser: P) -> Self {
        Self {
            parser,
            pending: [0; 4],
            pending_len: 0,
            consumed: 0,
        }
    }

    /// The parser behind this front end.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the progressive driver is not wired in yet")
    )]
    pub(crate) fn parser(&self) -> &P {
        &self.parser
    }

    /// Validates `bytes` and passes every complete character to the parser.
    ///
    /// A trailing partial character is kept until the next call. Invalid
    /// UTF-8 returns [`ParseError::Encoding`] with the offset counted from
    /// the start of the whole input, matching [`String::from_utf8`]'s message.
    pub(crate) fn feed(&mut self, mut bytes: &[u8]) -> Result<(), ParseError> {
        if self.pending_len > 0 {
            let width = utf8_width(self.pending[0]);
            let take = (width - self.pending_len).min(bytes.len());
            self.pending[self.pending_len..self.pending_len + take].copy_from_slice(&bytes[..take]);
            self.pending_len += take;
            bytes = &bytes[take..];
            if self.pending_len < width {
                return Ok(());
            }
            let pending = self.pending;
            self.pending_len = 0;
            self.push_valid(&pending[..width])?;
        }

        match std::str::from_utf8(bytes) {
            Ok(text) => {
                self.push_str(text);
                Ok(())
            }
            Err(error) => {
                let valid = error.valid_up_to();
                self.push_valid(&bytes[..valid])?;
                match error.error_len() {
                    Some(len) => Err(invalid_sequence(len, self.consumed)),
                    None => {
                        let rest = &bytes[valid..];
                        self.pending[..rest.len()].copy_from_slice(rest);
                        self.pending_len = rest.len();
                        Ok(())
                    }
                }
            }
        }
    }

    /// Reads `input` to its end, feeding each chunk as it arrives.
    ///
    /// Read failures return [`ParseError::Io`]; an [`ErrorKind::Interrupted`]
    /// read is retried, as [`Read::read_to_end`] does.
    pub(crate) fn feed_reader(&mut self, mut input: impl Read) -> Result<(), ParseError> {
        let mut buf = vec![0; READ_CHUNK_BYTES];
        loop {
            match input.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(read) => self.feed(&buf[..read])?,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(ParseError::Io(error)),
            }
        }
    }

    /// Ends the input and returns the parser's output.
    ///
    /// Input that stops inside a multi-byte character returns
    /// [`ParseError::Encoding`].
    pub(crate) fn finish(self) -> Result<P::Output, ParseError> {
        if self.pending_len > 0 {
            return Err(encoding_error(format!(
                "incomplete utf-8 byte sequence from index {}",
                self.consumed
            )));
        }
        Ok(self.parser.finish())
    }

    /// Passes bytes already known to end on a character boundary.
    fn push_valid(&mut self, bytes: &[u8]) -> Result<(), ParseError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| invalid_sequence(error.error_len().unwrap_or(1), self.consumed))?;
        self.push_str(text);
        Ok(())
    }

    fn push_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.parser.process(StrTendril::from_slice(text));
        self.consumed += text.len();
    }
}

/// Encoded length of the character that starts with `lead`.
///
/// Only called for a byte that [`std::str::from_utf8`] reported as the start
/// of an incomplete sequence, so `lead` is a valid multi-byte lead byte.
fn utf8_width(lead: u8) -> usize {
    match lead {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        _ => 2,
    }
}

fn invalid_sequence(len: usize, index: usize) -> ParseError {
    encoding_error(format!(
        "invalid utf-8 sequence of {len} bytes from index {index}"
    ))
}

fn encoding_error(reason: String) -> ParseError {
    ParseError::Encoding {
        label: String::from("utf-8"),
        reason,
    }
}

#[cfg(test)]
mod tests;
