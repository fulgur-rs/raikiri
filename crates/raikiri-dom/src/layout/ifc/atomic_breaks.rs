//! Soft wrap opportunities next to atomic inlines.
//!
//! CSS Text 3 §5.1 puts a soft wrap opportunity before and after each replaced
//! element or other atomic inline, even next to a character that Unicode line
//! breaking would keep it with: U+00A0 NO-BREAK SPACE and any punctuation
//! that may not start or end a line. Characters of the line breaking classes
//! GL, WJ and ZWJ other than U+00A0 still keep an atomic inline with them.
//!
//! The break analysis of the inline engine follows UAX #14, which treats the
//! object replacement character that stands for an atomic as a contingent
//! break. This is the override it lets the caller add on top of that.

use icu_properties::props::{BidiControl, LineBreak};
use icu_properties::{CodePointMapData, CodePointSetData};
use shodo::{LineBreakContext, LineBreakOverride, SoftBreakOpportunity};

/// The character that stands for an atomic inline in the text of a paragraph.
/// A literal U+FFFC in author text is an object by definition (UAX #14 gives
/// it the class CB, as it does the atomic's) and is treated the same.
const OBJECT: char = '\u{FFFC}';

/// Allows the soft wrap opportunity at `context.offset` when Unicode forbids
/// it, one side of it is an atomic inline, and the character on the other side
/// does not keep an atomic with it.
///
/// The bidi controls that the engine inserts around an isolate or an override
/// are no characters of the text: the cut stays before them, so the
/// neighbours are the characters beyond them.
///
/// The text of a paragraph shows an out-of-flow box (a float or an absolutely
/// positioned box) as the same character as an atomic inline, so a paragraph
/// that has one must not use this: the box is transparent to line breaking.
pub(super) fn around_atomics(context: LineBreakContext<'_>) -> LineBreakOverride {
    if context.standard != SoftBreakOpportunity::Prohibited {
        return LineBreakOverride::UseStandard;
    }
    let controls = CodePointSetData::new::<BidiControl>();
    let is_character = |c: &char| !controls.contains(*c);
    let before = context.text[..context.offset]
        .chars()
        .rev()
        .find(is_character);
    let after = context.text[context.offset..].chars().find(is_character);
    let neighbour = match (before, after) {
        (Some(OBJECT), Some(other)) | (Some(other), Some(OBJECT)) => other,
        _ => return LineBreakOverride::UseStandard,
    };
    if keeps_atomic(neighbour) {
        LineBreakOverride::UseStandard
    } else {
        LineBreakOverride::Allow
    }
}

/// Whether `c`, next to an atomic inline, leaves no soft wrap opportunity
/// between them.
///
/// Those are the glue classes of CSS Text 3 §5.1, except U+00A0, and the
/// combining marks: U+034F COMBINING GRAPHEME JOINER, which the specification
/// lists among the glue characters, has that class in current Unicode data.
/// The rest are left to the rules that already govern them: a space after an
/// atomic hangs at the end of its line instead of starting the next, and a
/// forced break ends the line without a soft break before it, which would
/// leave a line of nothing.
fn keeps_atomic(c: char) -> bool {
    match CodePointMapData::<LineBreak>::new().get(c) {
        LineBreak::Glue => c != '\u{a0}',
        LineBreak::WordJoiner
        | LineBreak::ZWJ
        | LineBreak::CombiningMark
        | LineBreak::Space
        | LineBreak::ZWSpace
        | LineBreak::MandatoryBreak
        | LineBreak::CarriageReturn
        | LineBreak::LineFeed
        | LineBreak::NextLine => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
