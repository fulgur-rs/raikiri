use super::*;
use html5ever::tendril::TendrilSink;

fn attribute(local: &str, value: &str) -> Attribute {
    Attribute {
        name: QualName::new(None, ns!(), local.into()),
        value: value.into(),
    }
}

/// The value of null-namespace attribute `local` on the first `<body>`.
fn body_attribute<'a>(document: &'a Document, local: &str) -> Option<&'a str> {
    document
        .preorder(document.root_index())
        .filter_map(|id| document.get_node(id))
        .find(|node| node.tag_name() == Some("body"))
        .expect("a body element")
        .attribute(local)
}

fn repeated_bodies(count: usize) -> String {
    let mut html = String::from("<!doctype html><html><body data-first='x'>");
    for index in 0..count {
        html.push_str(&format!("<body data-k{index}='v{index}'>"));
    }
    html.push_str("<p>text");
    html
}

#[test]
fn repeated_body_tags_keep_the_first_value_and_collect_the_new_names() {
    let html = "<!doctype html><html lang='en'><body class='a' data-k0='first'>\
                <body class='b' data-k0='second' data-k1='one'>\
                <body data-k1='two' data-k2='three'><p>text";
    let parser = html5ever::parse_document(RaikiriTreeSink::new(None), Default::default());
    let parsed = parser.one(html);
    let attribute = |local| body_attribute(&parsed.dom, local);
    assert_eq!(attribute("class"), Some("a"));
    assert_eq!(attribute("data-k0"), Some("first"));
    assert_eq!(attribute("data-k1"), Some("one"));
    assert_eq!(attribute("data-k2"), Some("three"));
}

#[test]
fn a_repeated_tag_reads_its_element_once_however_often_the_tag_repeats() {
    for count in [3, 300] {
        let mut parser = html5ever::parse_document(RaikiriTreeSink::new(None), Default::default());
        parser.process(repeated_bodies(count).into());
        // One name set for the body, built from the attributes it had when the
        // first repeated tag arrived, not one per repeated tag.
        assert_eq!(
            parser.tokenizer.sink.sink.attribute_name_sets_built.get(),
            1,
            "{count} repeated tags"
        );
        let parsed = parser.finish();
        assert_eq!(body_attribute(&parsed.dom, "data-first"), Some("x"));
        for index in 0..count {
            assert_eq!(
                body_attribute(&parsed.dom, &format!("data-k{index}")),
                Some(format!("v{index}").as_str()),
                "data-k{index} of {count}"
            );
        }
    }
}

#[test]
fn repeated_tags_do_not_add_nodes() {
    let count = |tags| {
        let parser = html5ever::parse_document(RaikiriTreeSink::new(None), Default::default());
        parser.one(repeated_bodies(tags)).dom.node_count()
    };
    assert_eq!(count(3), count(300));
}

#[test]
fn names_differing_in_namespace_or_prefix_are_distinct_attributes() {
    let sink = RaikiriTreeSink::new(None);
    let svg = sink.create_element(
        QualName::new(None, ns!(svg), "a".into()),
        vec![Attribute {
            name: QualName::new(Some("xlink".into()), ns!(xlink), "href".into()),
            value: "#one".into(),
        }],
        ElementFlags::default(),
    );
    sink.add_attrs_if_missing(
        &svg,
        vec![
            attribute("href", "plain"),
            Attribute {
                name: QualName::new(Some("xlink".into()), ns!(xlink), "href".into()),
                value: "#again".into(),
            },
        ],
    );
    let store = sink.attributes.borrow();
    let values: Vec<(&str, &str)> = store[&svg]
        .iter()
        .map(|a| (a.name.local.as_ref(), a.value.as_ref()))
        .collect();
    assert_eq!(values, [("href", "#one"), ("href", "plain")]);
    assert_eq!(sink.attribute_name_sets_built.get(), 1);
}

#[test]
fn attributes_arriving_after_pages_were_delivered_are_not_remembered() {
    let sink = RaikiriTreeSink::new(None);
    let body = sink.create_element(
        QualName::new(None, ns!(html), "body".into()),
        vec![attribute("class", "a")],
        ElementFlags::default(),
    );
    sink.add_attrs_if_missing(&body, vec![attribute("data-early", "1")]);
    sink.freeze_root_attributes();
    sink.add_attrs_if_missing(&body, vec![attribute("data-late", "2")]);
    sink.add_attrs_if_missing(&body, vec![attribute("data-later", "3")]);
    let store = sink.attributes.borrow();
    let names: Vec<&str> = store[&body].iter().map(|a| a.name.local.as_ref()).collect();
    assert_eq!(names, ["class", "data-early"]);
    // One warning for the element, however many tags repeated after delivery.
    assert_eq!(sink.warnings.borrow().len(), 1);
    assert_eq!(sink.attribute_name_sets_built.get(), 1);
}
