use std::borrow::Cow;
use std::io::{Error, ErrorKind, Read};

use html5ever::tendril::fmt::UTF8;
use html5ever::tendril::{StrTendril, TendrilSink};
use raikiri_traits::{
    Dom, Element, LimitKind, Node, NodeKind, ParseError, RenderError, RenderLimits,
};

use super::Utf8Feed;
use crate::{ParseOptions, parse, parse_html_with_limits};

/// Parser stand-in that records the text it receives.
#[derive(Default)]
struct Recorder {
    text: String,
    pieces: usize,
}

impl TendrilSink<UTF8> for Recorder {
    type Output = (String, usize);

    fn process(&mut self, t: StrTendril) {
        self.text.push_str(&t);
        self.pieces += 1;
    }

    fn error(&mut self, _desc: Cow<'static, str>) {}

    fn finish(self) -> Self::Output {
        (self.text, self.pieces)
    }
}

fn feed_chunks(chunks: &[&[u8]]) -> Result<String, ParseError> {
    let mut feed = Utf8Feed::new(Recorder::default());
    for chunk in chunks {
        feed.feed(chunk)?;
    }
    feed.finish().map(|(text, _)| text)
}

fn encoding_reason(error: ParseError) -> String {
    match error {
        ParseError::Encoding { label, reason } => {
            assert_eq!(label, "utf-8");
            reason
        }
        other => panic!("expected Encoding, got {other:?}"),
    }
}

/// Reader that hands out its input `step` bytes at a time.
struct Trickle<'a> {
    input: &'a [u8],
    step: usize,
}

impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.step.min(buf.len()).min(self.input.len());
        buf[..n].copy_from_slice(&self.input[..n]);
        self.input = &self.input[n..];
        Ok(n)
    }
}

fn options<'a>() -> ParseOptions<'a> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

/// Element tags and text of the tree, in document order.
fn outline(doc: &raikiri_dom::Document) -> String {
    fn walk(doc: &raikiri_dom::Document, id: raikiri_traits::NodeId, out: &mut String) {
        let Some(node) = doc.node(id) else { return };
        match node.kind() {
            NodeKind::Element => {
                out.push('<');
                if let Some(el) = node.as_element() {
                    out.push_str(el.tag_name());
                }
                out.push('>');
                for child in doc.child_ids(id) {
                    walk(doc, child, out);
                }
                out.push_str("</>");
            }
            NodeKind::Text => {
                out.push('[');
                out.push_str(node.text_content().unwrap_or(""));
                out.push(']');
            }
            _ => {
                for child in doc.child_ids(id) {
                    walk(doc, child, out);
                }
            }
        }
    }
    let mut out = String::new();
    walk(doc, doc.root_id(), &mut out);
    out
}

#[test]
fn characters_split_at_every_byte_reassemble() {
    let input = "a日本語😀é<b>".as_bytes();
    for split in 0..=input.len() {
        let text = feed_chunks(&[&input[..split], &input[split..]]).expect("valid utf-8");
        assert_eq!(text.as_bytes(), input, "split at {split}");
    }
    let bytes: Vec<&[u8]> = input.chunks(1).collect();
    assert_eq!(feed_chunks(&bytes).expect("valid utf-8").as_bytes(), input);
}

#[test]
fn complete_characters_reach_the_parser_without_waiting_for_more_input() {
    let mut feed = Utf8Feed::new(Recorder::default());
    feed.feed("ab日".as_bytes()).unwrap();
    feed.feed(&"本".as_bytes()[..1]).unwrap();
    let (text, pieces) = feed.finish_for_test();
    assert_eq!(text, "ab日");
    assert_eq!(pieces, 1);
}

#[test]
fn encoding_errors_match_whole_input_messages() {
    let cases: &[&[u8]] = &[
        b"abc\xFFdef",
        b"\xE6\x97x",
        b"ok\xF0\x9F\x98",
        b"\xC3",
        b"x\xED\xA0\x80",
    ];
    for input in cases {
        let expected = String::from_utf8(input.to_vec()).unwrap_err().to_string();
        for split in 0..=input.len() {
            let error = feed_chunks(&[&input[..split], &input[split..]])
                .expect_err("invalid utf-8 must fail");
            assert_eq!(
                encoding_reason(error),
                expected,
                "input {input:?} split at {split}"
            );
        }
    }
}

#[test]
fn feed_reader_retries_interrupted_reads() {
    struct Interrupting {
        interrupted: bool,
        rest: &'static [u8],
    }
    impl Read for Interrupting {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(Error::from(ErrorKind::Interrupted));
            }
            let n = buf.len().min(self.rest.len());
            buf[..n].copy_from_slice(&self.rest[..n]);
            self.rest = &self.rest[n..];
            Ok(n)
        }
    }

    let mut feed = Utf8Feed::new(Recorder::default());
    feed.feed_reader(Interrupting {
        interrupted: false,
        rest: b"hello",
    })
    .expect("interrupted read is retried");
    assert_eq!(feed.finish().unwrap().0, "hello");
}

#[test]
fn trickled_input_builds_the_same_tree() {
    let html = "<!doctype html><html><head><style>p{color:red}</style></head>\
                <body><p>Hello, 世界 <b>bold <i>both</b> italic</i></p>\
                <table><tr><td>cell</td>stray</tr></table>\
                <pre>\r\nline1\r\nline2</pre><!-- note --></body></html>";
    let whole = parse(html.as_bytes(), &options()).expect("whole input parses");
    for step in [1, 2, 3, 7, 64] {
        let trickled = parse(
            Trickle {
                input: html.as_bytes(),
                step,
            },
            &options(),
        )
        .expect("trickled input parses");
        assert_eq!(outline(&trickled.dom), outline(&whole.dom), "step {step}");
        assert_eq!(
            trickled.stylesheet_sources.len(),
            whole.stylesheet_sources.len()
        );
    }
}

#[test]
fn text_split_across_reads_stays_one_text_node() {
    let html = "<p>abcdefghij</p>";
    let doc = parse(
        Trickle {
            input: html.as_bytes(),
            step: 1,
        },
        &options(),
    )
    .expect("parse");
    assert_eq!(
        outline(&doc.dom),
        "<html><head></><body><p>[abcdefghij]</></></>"
    );
}

#[test]
fn text_gets_its_node_id_before_the_next_element_is_created() {
    let doc = parse("<p>lead<b>x</b></p>".as_bytes(), &options()).expect("parse");
    let mut lead = None;
    let mut bold = None;
    for id in (0..doc.dom.node_count() as u64).map(raikiri_traits::NodeId) {
        let Some(node) = doc.dom.node(id) else {
            continue;
        };
        if node.text_content() == Some("lead") {
            lead = Some(id.0);
        }
        if node.as_element().is_some_and(|el| el.tag_name() == "b") {
            bold = Some(id.0);
        }
    }
    assert!(
        lead.expect("text") < bold.expect("element"),
        "ids follow the tree builder's order"
    );
}

#[test]
fn foster_parented_text_merges_with_the_text_before_the_table() {
    let html = "<div>lead<table>a&amp;b<tr><td>c</td></tr></table></div>";
    let doc = parse(html.as_bytes(), &options()).expect("parse");
    assert_eq!(
        outline(&doc.dom),
        "<html><head></><body><div>[leada&b]<table><tbody><tr><td>[c]</></></></></></></>"
    );
}

#[test]
fn input_cap_stops_reading_an_endless_reader() {
    let limits = RenderLimits::builder().max_input_bytes(Some(100)).build();
    let error = parse_html_with_limits(std::io::repeat(b'a'), &options(), limits)
        .expect_err("endless input exceeds the cap");
    match error {
        RenderError::LimitExceeded {
            kind,
            limit,
            actual,
        } => {
            assert_eq!(kind, LimitKind::InputBytes);
            assert_eq!(limit, 100);
            assert_eq!(actual, 101);
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

#[test]
fn input_cap_reports_exceeded_bytes_even_after_invalid_trailing_utf8() {
    // The cap is checked before the chunk that crosses it is decoded.
    let limits = RenderLimits::builder().max_input_bytes(Some(4)).build();
    let error =
        parse_html_with_limits(&b"abcd\xFF"[..], &options(), limits).expect_err("over the cap");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::InputBytes,
            actual: 5,
            ..
        }
    ));
}

impl<P: TendrilSink<UTF8>> Utf8Feed<P> {
    /// Finishes without checking for a pending partial character.
    fn finish_for_test(self) -> P::Output {
        self.parser.finish()
    }
}
