use std::borrow::Cow;

use html5ever::tendril::fmt::UTF8;
use html5ever::tendril::{StrTendril, TendrilSink};

use super::SkipInternalSubset;

/// Parser stand-in that records the text and the errors it receives.
#[derive(Default)]
struct Recorder {
    text: String,
    errors: Vec<String>,
}

impl TendrilSink<UTF8> for Recorder {
    type Output = (String, Vec<String>);

    fn process(&mut self, t: StrTendril) {
        self.text.push_str(&t);
    }

    fn error(&mut self, desc: Cow<'static, str>) {
        self.errors.push(desc.into_owned());
    }

    fn finish(self) -> Self::Output {
        (self.text, self.errors)
    }
}

/// What the parser receives for `chunks` fed one after the other.
fn run(chunks: &[&str]) -> (String, Vec<String>) {
    let mut skipper = SkipInternalSubset::new(Recorder::default());
    for chunk in chunks {
        skipper.process(StrTendril::from_slice(chunk));
    }
    skipper.finish()
}

const IGNORED: &str = "Internal DTD subset ignored";

/// Feeds `source` whole, then in every two-piece split and one character at a
/// time, expecting the same output as for the whole input each time.
fn assert_chunking_changes_nothing(source: &str) -> (String, Vec<String>) {
    let whole = run(&[source]);
    for at in (0..=source.len()).filter(|&at| source.is_char_boundary(at)) {
        assert_eq!(run(&[&source[..at], &source[at..]]), whole, "split at {at}");
    }
    let pieces: Vec<String> = source.chars().map(String::from).collect();
    let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
    assert_eq!(run(&pieces), whole, "one character at a time");
    whole
}

#[test]
fn input_without_a_doctype_passes_through() {
    let source = "<?xml version=\"1.0\"?>\n<!-- note -->\n<html>[x]</html>";
    assert_eq!(
        assert_chunking_changes_nothing(source),
        (source.into(), vec![])
    );
}

#[test]
fn a_doctype_without_an_internal_subset_passes_through() {
    for source in [
        "<!DOCTYPE html><html/>",
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\"><html/>",
        // Brackets and `>` inside literals are not a subset.
        "<!DOCTYPE html SYSTEM \"http://example.invalid/[unused].dtd\"><html/>",
        "<!DOCTYPE html SYSTEM 'a>[b'><html/>",
    ] {
        assert_eq!(
            assert_chunking_changes_nothing(source),
            (source.into(), vec![])
        );
    }
}

#[test]
fn the_internal_subset_is_dropped_and_reported_once() {
    let (text, errors) = assert_chunking_changes_nothing(
        "<!DOCTYPE html [<!ENTITY a \"x\"> <!ENTITY b \"y\">]><html/>",
    );
    assert_eq!(text, "<!DOCTYPE html ><html/>");
    assert_eq!(errors, [IGNORED]);
}

#[test]
fn the_subset_follows_public_and_system_identifiers() {
    let (text, errors) = assert_chunking_changes_nothing(
        "<!DOCTYPE html PUBLIC \"-//X//EN\" \"x.dtd\" [<!ENTITY a \"x\">] ><html/>",
    );
    assert_eq!(
        text,
        "<!DOCTYPE html PUBLIC \"-//X//EN\" \"x.dtd\"  ><html/>"
    );
    assert_eq!(errors, [IGNORED]);
}

#[test]
fn quotes_comments_and_instructions_hide_what_would_end_the_subset() {
    let (text, errors) = assert_chunking_changes_nothing(concat!(
        "<!DOCTYPE html [",
        "<!ENTITY a \"]>\"> <!ENTITY b ']'>",
        "<!-- ] > \" -->",
        "<!--- ] --->",
        "<?pi ] > ' ?>",
        "<?pi ??>",
        "<!ENTITY c \"<x/>\">",
        "<!ENTITY é \"é]\">",
        "]><html/>",
    ));
    assert_eq!(text, "<!DOCTYPE html ><html/>");
    assert_eq!(errors, [IGNORED]);
}

#[test]
fn markup_that_looks_like_a_subset_in_the_prolog_is_not_one() {
    for source in [
        "<?pi <!DOCTYPE html [?><!-- <!DOCTYPE html [ --><html/>",
        "<!-- <!DOCTYPE x [ --><!DOCTYPE html><html>[</html>",
    ] {
        assert_eq!(
            assert_chunking_changes_nothing(source),
            (source.into(), vec![])
        );
    }
}

#[test]
fn scanning_stops_at_the_end_of_the_prolog() {
    for source in [
        // After the root starts, `<!DOCTYPE` is only an error for the parser.
        "<root><!DOCTYPE html [a]></root>",
        "<root/><!DOCTYPE html [a]>",
        // After the DOCTYPE ends.
        "<!DOCTYPE html><!DOCTYPE html [a]><html/>",
        // CDATA and other declarations are not prolog items.
        "<![CDATA[<!DOCTYPE html [a]>]]>",
        "<!ENTITY a><!DOCTYPE html [a]>",
        "</x><!DOCTYPE html [a]>",
    ] {
        assert_eq!(
            assert_chunking_changes_nothing(source),
            (source.into(), vec![])
        );
    }
}

#[test]
fn an_unterminated_subset_swallows_the_rest_and_is_reported() {
    let (text, errors) =
        assert_chunking_changes_nothing("<!DOCTYPE html [<!ENTITY a \"x\"><html/>");
    assert_eq!(text, "<!DOCTYPE html ");
    assert_eq!(errors, [IGNORED]);
}

#[test]
fn errors_from_the_decoder_reach_the_parser() {
    let mut skipper = SkipInternalSubset::new(Recorder::default());
    skipper.error(Cow::Borrowed("bad byte"));
    assert_eq!(skipper.finish().1, ["bad byte"]);
}

#[test]
fn a_subset_spanning_chunks_is_reported_once() {
    let (text, errors) = run(&["<!DOCTYPE html [", "<!ENTITY a \"x\">", "]>", "<html/>"]);
    assert_eq!(text, "<!DOCTYPE html ><html/>");
    assert_eq!(errors, [IGNORED]);
}

#[test]
fn a_marker_that_goes_wrong_ends_the_scan_or_the_declaration() {
    // `<!DOCTYPX` is not a DOCTYPE and `<!-x` is not a comment: the scan ends
    // and the rest of the input passes through.
    for source in ["<!DOCTYPX html [a]>", "<!-x [a]>"] {
        assert_eq!(
            assert_chunking_changes_nothing(source),
            (source.into(), vec![])
        );
    }
    // Inside the subset, `<!-` that does not open a comment falls back to
    // scanning declarations, so the `]` still closes the subset.
    assert_eq!(
        assert_chunking_changes_nothing("<!DOCTYPE html [<!-]><root/>"),
        ("<!DOCTYPE html ><root/>".into(), vec![IGNORED.into()])
    );
}
