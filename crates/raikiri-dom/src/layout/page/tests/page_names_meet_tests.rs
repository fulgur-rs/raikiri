use super::*;

/// Lays out a body built by `build`, which returns the two boxes to compare,
/// and answers whether their page values meet between them. `sheet` is a
/// style sheet for the document.
fn meet(sheet: &str, build: impl FnOnce(&mut Document, usize) -> (usize, usize)) -> bool {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    if !sheet.is_empty() {
        let style = doc.append_element(Some(0), "style", None::<&str>);
        doc.append_text(style, sheet);
    }
    let (before, after) = build(&mut doc, body);
    doc.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).expect("cascade");
    layout_pages(&mut doc, &cascade, PageBox::A4).expect("layout");
    page_names_meet(&doc, &cascade, before, after, false)
}

fn div(doc: &mut Document, parent: usize, css: &str) -> usize {
    let id = doc.append_element(Some(parent), "div", Some(css));
    doc.append_text(id, "x");
    id
}

fn empty(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(css))
}

/// Two sibling blocks in a block of `parent_css`, the first holding a last
/// child of `last_css` and the second named by `next_css`.
fn pair(parent_css: &str, first_css: &str, last_css: &str, next_css: &str) -> bool {
    meet("", |doc, body| {
        let parent = empty(doc, body, parent_css);
        let first = empty(doc, parent, first_css);
        div(doc, first, last_css);
        (first, div(doc, parent, next_css))
    })
}

#[test]
fn boxes_meet_when_the_end_value_of_one_is_the_start_value_of_the_next() {
    assert!(pair(
        "display:block",
        "display:block",
        "display:block",
        "display:block"
    ));
    assert!(!pair(
        "display:block",
        "display:block",
        "display:block",
        "display:block;page:b"
    ));
    // The end value is that of the deepest last child.
    assert!(pair(
        "display:block",
        "display:block",
        "display:block;page:x",
        "display:block;page:x"
    ));
    assert!(!pair(
        "display:block",
        "display:block",
        "display:block;page:x",
        "display:block"
    ));
    // One value around both boxes.
    assert!(pair(
        "display:block;page:a",
        "display:block",
        "display:block",
        "display:block"
    ));
    // A monolithic box ends on its own page value whatever it holds.
    assert!(!pair(
        "display:block",
        "display:block;contain:size;height:10px",
        "display:block;page:x",
        "display:block;page:x"
    ));
}

#[test]
fn content_after_a_named_child_ends_the_box_on_its_own_value() {
    // Text after the named child.
    assert!(!meet("", |doc, body| {
        let first = empty(doc, body, "display:block");
        div(doc, first, "display:block;page:x");
        doc.append_text(first, "tail");
        (first, div(doc, body, "display:block;page:x"))
    }));
    // A generated box after it.
    assert!(!meet(
        "section::after { content:'M'; display:block }",
        |doc, body| {
            let first = doc.append_element(Some(body), "section", Some("display:block"));
            div(doc, first, "display:block;page:x");
            (first, div(doc, body, "display:block;page:x"))
        }
    ));
    // An empty box passes up nothing: its own value ends it.
    assert!(meet("", |doc, body| {
        let first = empty(doc, body, "display:block;height:10px");
        (first, div(doc, body, "display:block"))
    }));
}

#[test]
fn a_box_without_content_starts_on_its_inherited_value() {
    assert!(meet("", |doc, body| {
        let parent = empty(doc, body, "display:block;page:a");
        let first = div(doc, parent, "display:block");
        (first, empty(doc, parent, "display:block;height:10px"))
    }));
}

#[test]
fn a_box_that_does_not_stack_its_children_ends_on_its_start_value() {
    // The first item of a row flex container names the page of its line,
    // which the container ends on too.
    assert!(meet("", |doc, body| {
        let flex = empty(doc, body, "display:flex");
        let item = empty(doc, flex, "display:block");
        div(doc, item, "display:block;page:x");
        div(doc, flex, "display:block");
        (flex, div(doc, body, "display:block;page:x"))
    }));
}

#[test]
fn names_that_pagination_ignores_always_meet() {
    for parent_css in [
        "display:flex;flex-direction:column",
        "display:grid",
        "display:block;columns:2",
        "display:block;float:left;width:50px",
        "display:block;position:absolute",
    ] {
        assert!(
            pair(
                parent_css,
                "display:block;page:a",
                "display:block",
                "display:block;page:b"
            ),
            "{parent_css}"
        );
    }
}

#[test]
fn a_multicol_container_keeps_its_own_page_value() {
    // Its own value starts or ends a page; the names in its columns do not.
    assert!(!pair(
        "display:block",
        "display:block",
        "display:block",
        "display:block;columns:2;page:b"
    ));
    assert!(!meet("", |doc, body| {
        let first = div(doc, body, "display:block;columns:2;page:a");
        (first, div(doc, body, "display:block"))
    }));
    assert!(meet("", |doc, body| {
        let first = empty(doc, body, "display:block;columns:2");
        div(doc, first, "display:block;page:x");
        (first, div(doc, body, "display:block"))
    }));
}

#[test]
fn a_contents_element_that_puts_nothing_on_the_page_passes_no_value() {
    // An empty `display: contents` element after the last box, or before the
    // first one, is not where the page value ends or starts.
    assert!(!meet("", |doc, body| {
        let first = empty(doc, body, "display:block");
        div(doc, first, "display:block;page:x");
        let contents = empty(doc, first, "display:contents");
        empty(doc, contents, "display:contents");
        doc.append_text(contents, " ");
        empty(doc, contents, "display:none");
        (first, div(doc, body, "display:block"))
    }));
    assert!(!meet("", |doc, body| {
        let first = div(doc, body, "display:block");
        let second = empty(doc, body, "display:block");
        empty(doc, second, "display:contents");
        div(doc, second, "display:block;page:x");
        (first, second)
    }));
    // One that holds a box passes its value on.
    assert!(meet("", |doc, body| {
        let first = empty(doc, body, "display:block");
        div(doc, first, "display:block;page:x");
        let contents = empty(doc, first, "display:contents");
        div(doc, contents, "display:block");
        (first, div(doc, body, "display:block"))
    }));
}

#[test]
fn the_own_names_of_flex_items_pass_to_no_container() {
    for container in [
        "display:flex",
        "display:flex;flex-direction:column",
        "display:grid",
    ] {
        let flex = container != "display:grid";
        // A flex item's own value does not start the container; a grid
        // item's does.
        assert_eq!(
            flex,
            meet("", |doc, body| {
                let first = div(doc, body, "display:block");
                let second = empty(doc, body, container);
                div(doc, second, "display:block;page:x");
                (first, second)
            })
        );
        // Nor does it end it.
        assert_eq!(
            flex,
            meet("", |doc, body| {
                let first = empty(doc, body, container);
                div(doc, first, "display:block;page:x");
                (first, div(doc, body, "display:block"))
            })
        );
        // A box nested in an item still passes its value up.
        assert!(!meet("", |doc, body| {
            let first = div(doc, body, "display:block");
            let second = empty(doc, body, container);
            let item = empty(doc, second, "display:block");
            div(doc, item, "display:block;page:x");
            (first, second)
        }));
    }
}

#[test]
fn generated_content_puts_a_contents_element_on_the_page() {
    // A `display: contents` element whose `::before` lays out a box starts
    // the page value of its parent, unnamed.
    assert!(meet(
        "span::before{content:'M';display:block}",
        |doc, body| {
            let first = div(doc, body, "display:block");
            let second = empty(doc, body, "display:block");
            doc.append_element(Some(second), "span", Some("display:contents"));
            div(doc, second, "display:block;page:x");
            (first, second)
        }
    ));
    // A comment holds nothing for the page.
    assert!(!meet("", |doc, body| {
        let first = div(doc, body, "display:block");
        let second = empty(doc, body, "display:block");
        let contents = empty(doc, second, "display:contents");
        doc.append_comment(Some(contents), "note");
        div(doc, second, "display:block;page:x");
        (first, second)
    }));
}
