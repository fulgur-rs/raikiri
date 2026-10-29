use super::*;
use html5ever::tendril::TendrilSink;

fn element(sink: &RaikiriTreeSink, tag: &str) -> usize {
    sink.create_element(
        QualName::new(None, ns!(html), tag.into()),
        Vec::new(),
        ElementFlags::default(),
    )
}

fn check(sink: &RaikiriTreeSink, previous: Option<Document>) -> (Document, usize) {
    let (actual, copied) = sink.snapshot_reusing(previous);
    let expected = sink.snapshot();
    assert_eq!(actual.dom.node_count(), expected.dom.node_count());
    for id in 0..expected.dom.node_count() {
        assert_eq!(
            format!("{:?}", actual.dom.get_node(id)),
            format!("{:?}", expected.dom.get_node(id)),
            "node {id}"
        );
    }
    assert_eq!(
        format!("{:?}", actual.stylesheet_sources),
        format!("{:?}", expected.stylesheet_sources)
    );
    assert_eq!(
        format!("{:?}", actual.warnings),
        format!("{:?}", expected.warnings)
    );
    (actual.dom, copied)
}

#[test]
fn source_snapshot_reuses_an_untouched_prefix() {
    let sink = RaikiriTreeSink::new(None);
    let root = sink.get_document();
    let body = element(&sink, "body");
    sink.append(&root, NodeOrText::AppendNode(body));
    for _ in 0..200 {
        let p = element(&sink, "p");
        sink.append(&body, NodeOrText::AppendNode(p));
        sink.append(&p, NodeOrText::AppendText("old prefix".into()));
    }
    let (mut previous, _) = check(&sink, None);
    for _ in 0..8 {
        let p = element(&sink, "p");
        sink.append(&body, NodeOrText::AppendNode(p));
        sink.append(&p, NodeOrText::AppendText("new suffix".into()));
        let (next, copied) = check(&sink, Some(previous));
        assert!(
            copied <= 5,
            "copied {copied} nodes of {}",
            next.node_count()
        );
        previous = next;
    }
}

#[test]
fn source_snapshot_preserves_direct_callback_topology_and_text_changes() {
    let sink = RaikiriTreeSink::new(None);
    let root = sink.get_document();
    let a = element(&sink, "div");
    let b = element(&sink, "div");
    let c = element(&sink, "span");
    sink.append(&root, NodeOrText::AppendNode(a));
    sink.append(&root, NodeOrText::AppendNode(b));
    sink.append(&a, NodeOrText::AppendNode(c));
    sink.append(&c, NodeOrText::AppendText("first".into()));
    let (previous, _) = check(&sink, None);
    // Existing callbacks do not detach on append; preserve both child lists.
    sink.append(&b, NodeOrText::AppendNode(c));
    sink.append(&a, NodeOrText::AppendNode(c));
    sink.append(&c, NodeOrText::AppendText(" second".into()));
    let (previous, _) = check(&sink, Some(previous));
    sink.remove_from_parent(&c);
    sink.reparent_children(&b, &a);
    sink.append_before_sibling(&c, NodeOrText::AppendText("before ".into()));
    let (previous, _) = check(&sink, Some(previous));
    sink.append_before_sibling(&c, NodeOrText::AppendText("merged ".into()));
    let _ = check(&sink, Some(previous));
}

#[test]
fn source_snapshot_tracks_fragment_drains_and_same_parent_reordering() {
    let sink = RaikiriTreeSink::new(None);
    let root = sink.get_document();
    let body = element(&sink, "body");
    sink.append(&root, NodeOrText::AppendNode(body));
    let mut flags = ElementFlags::default();
    flags.template = true;
    let template = sink.create_element(
        QualName::new(None, ns!(html), "template".into()),
        Vec::new(),
        flags,
    );
    let fragment = sink.get_template_contents(&template);
    let a = element(&sink, "p");
    let b = element(&sink, "p");
    sink.append(&fragment, NodeOrText::AppendNode(a));
    sink.append(&body, NodeOrText::AppendNode(b));
    let (previous, _) = check(&sink, None);
    sink.append(&body, NodeOrText::AppendNode(fragment));
    let (previous, _) = check(&sink, Some(previous));
    sink.append_before_sibling(&b, NodeOrText::AppendNode(a));
    let (previous, _) = check(&sink, Some(previous));
    sink.remove_from_parent(&a);
    sink.append(&body, NodeOrText::AppendNode(a));
    let _ = check(&sink, Some(previous));
}

#[test]
fn source_snapshot_matches_fresh_parsing_at_each_checkpoint() {
    let cases = [
        "<!doctype html><style>p { color: red }</style><body><p><b>one<i>two</b>three</i><p>four",
        "<!doctype html><body><table>foster<tr><td><b>cell</table>after</b><p>end",
        "<!doctype html><body><template><table><tr><td>inert</template><p>visible<!-- comment -->",
        "<!doctype html><body><svg xmlns='http://www.w3.org/2000/svg'><a xlink:href='#x'>a</a></svg><math><mi>x</mi></math>",
        "<!doctype html><html><body><p>text<body class='later' style='color:red'><html lang='en'>more",
    ];
    for html in cases {
        for chunk_size in [1, 7, 31] {
            let parser = html5ever::parse_document(RaikiriTreeSink::new(None), Default::default());
            let mut parser = parser;
            let mut previous = None;
            for (index, chunk) in html.as_bytes().chunks(chunk_size).enumerate() {
                parser.process(std::str::from_utf8(chunk).expect("ASCII fixture").into());
                let sink = &parser.tokenizer.sink.sink;
                let (next, _) = check(sink, previous.take());
                // A checkpoint that cannot recycle must restart the epoch.
                if index % 5 != 2 {
                    previous = Some(next);
                }
            }
        }
    }
}
