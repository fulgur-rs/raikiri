use super::*;
use crate::LayoutStyle as Style;

/// `body > div (container) > wrapper > [text,] spanner [, text]`, laid out.
/// Returns the document and the wrapper.
fn wrapped_spanner(container_css: &str, wrapper_tag: &str, wrapper_css: &str) -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let container = doc.append_element(Some(body), "div", Style::default(), Some(container_css));
    let wrapper = doc.append_element(
        Some(container),
        wrapper_tag,
        Style::default(),
        Some(wrapper_css),
    );
    doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:block;column-span:all;height:10px"),
    );
    lay_out(&mut doc);
    (doc, wrapper)
}

fn lay_out(doc: &mut Document) {
    let rules = raikiri_style::build_rule_tree(doc);
    let cascade = raikiri_style::cascade(doc, &rules).expect("cascade");
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(doc, &cascade, page).expect("layout");
}

const COLUMNS: &str = "display:block;width:100px;columns:2;column-gap:0";
const BLOCK: &str = "display:block";

#[test]
fn a_plain_wrapper_of_a_spanner_makes_no_box() {
    for css in [
        "display:block",
        "display:block;margin:0 auto",
        "display:block;color:red;font-weight:bold",
        "display:block;visibility:hidden",
    ] {
        let (doc, wrapper) = wrapped_spanner(COLUMNS, "div", css);
        assert!(doc.nodes[wrapper].generates_no_box(), "{css}");
    }
}

#[test]
fn every_plain_wrapper_between_the_spanner_and_the_container_makes_no_box() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let container = doc.append_element(Some(body), "div", Style::default(), Some(COLUMNS));
    let outer = doc.append_element(Some(container), "div", Style::default(), Some(BLOCK));
    let contents = doc.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("display:contents"),
    );
    let inner = doc.append_element(Some(contents), "section", Style::default(), Some(BLOCK));
    let spanner = doc.append_element(
        Some(inner),
        "h2",
        Style::default(),
        Some("display:block;column-span:all;height:10px"),
    );
    let beside = doc.append_element(Some(container), "div", Style::default(), Some(BLOCK));
    doc.append_element(Some(beside), "p", Style::default(), Some(BLOCK));
    lay_out(&mut doc);

    assert!(doc.nodes[outer].generates_no_box());
    assert!(doc.nodes[inner].generates_no_box());
    assert!(!doc.nodes[container].generates_no_box());
    assert!(!doc.nodes[spanner].generates_no_box());
    // A wrapper that holds no spanner stays a block.
    assert!(!doc.nodes[beside].generates_no_box());
}

#[test]
fn a_wrapper_with_a_box_of_its_own_keeps_it() {
    for css in [
        "padding-top:1px",
        "border-top:1px solid",
        "margin-top:5px",
        "margin-left:5px",
        "width:50px",
        "height:10px",
        "min-height:10px",
        "max-width:50px",
        "position:relative",
        "position:absolute",
        "position:fixed",
        "position:sticky",
        "float:left",
        "clear:both",
        "overflow:hidden",
        "background:red",
        "background-image:linear-gradient(red,blue)",
        "opacity:.5",
        "transform:translate(1px)",
        "filter:blur(1px)",
        "outline:1px solid",
        "box-shadow:1px 1px black",
        "isolation:isolate",
        "will-change:position",
        "contain:paint",
        "break-before:column",
        "break-after:column",
        "break-inside:avoid",
        "writing-mode:vertical-rl",
        "columns:2",
        "display:flow-root",
        "display:flex",
        "display:grid",
        "display:table",
        "display:list-item",
        "display:none",
    ] {
        let (doc, wrapper) = wrapped_spanner(COLUMNS, "div", &format!("{BLOCK};{css}"));
        assert!(!doc.nodes[wrapper].generates_no_box(), "{css}");
    }
    for tag in [
        "fieldset", "details", "dialog", "legend", "summary", "canvas", "img",
    ] {
        let (doc, wrapper) = wrapped_spanner(COLUMNS, tag, "display:block");
        assert!(!doc.nodes[wrapper].generates_no_box(), "{tag}");
    }
}

#[test]
fn a_wrapper_below_one_that_keeps_its_box_is_not_reached() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let container = doc.append_element(Some(body), "div", Style::default(), Some(COLUMNS));
    let outer = doc.append_element(Some(container), "div", Style::default(), Some(BLOCK));
    let padded = doc.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("display:block;padding:1px"),
    );
    let inner = doc.append_element(Some(padded), "div", Style::default(), Some(BLOCK));
    doc.append_element(
        Some(inner),
        "h2",
        Style::default(),
        Some("display:block;column-span:all;height:10px"),
    );
    lay_out(&mut doc);

    // The spanner spans the columns of the padded wrapper, if of any, not
    // those of the container: nothing between is lifted.
    for id in [outer, padded, inner] {
        assert!(!doc.nodes[id].generates_no_box());
    }
}

/// Whether a plain wrapper that holds `content` and a spanner, in a container
/// styled with `container_css`, makes no box. `content` is `text`, `inline`
/// (an inline-level box), `space` (white space only), `nbsp` (a no-break
/// space) or `comment`.
fn lifts_with_lines(container_css: &str, wrapper_css: &str, content: &str) -> bool {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let container = doc.append_element(Some(body), "div", Style::default(), Some(container_css));
    let wrapper = doc.append_element(Some(container), "div", Style::default(), Some(wrapper_css));
    match content {
        "text" => {
            doc.append_text(wrapper, "before");
        }
        "inline" => {
            doc.append_element(Some(wrapper), "b", Style::default(), None::<&str>);
        }
        "space" => {
            doc.append_text(wrapper, " \n ");
        }
        "comment" => {
            doc.append_comment(Some(wrapper), "note");
        }
        "nbsp" => {
            doc.append_text(wrapper, "\u{a0}");
        }
        _ => unreachable!(),
    }
    doc.append_element(
        Some(wrapper),
        "h2",
        Style::default(),
        Some("display:block;column-span:all;height:10px"),
    );
    lay_out(&mut doc);
    doc.nodes[wrapper].generates_no_box()
}

#[test]
fn the_lines_a_wrapper_holds_keep_their_layout() {
    // Lines laid out as in the container.
    assert!(lifts_with_lines(COLUMNS, BLOCK, "text"));
    assert!(lifts_with_lines(COLUMNS, BLOCK, "inline"));
    // What the container does not do to its lines the wrapper must not do
    // either: the lines would take the container's layout and style.
    for css in [
        "text-align:center",
        "text-align-last:right",
        "text-justify:inter-character",
        "text-indent:10px",
        "text-indent:10px hanging",
        "text-indent:10px each-line",
        "direction:rtl",
        "unicode-bidi:bidi-override",
        "orphans:3",
        "widows:3",
        "font-family:Ahem",
        "font-size:30px",
        "font-weight:bold",
        "font-style:italic",
        "font-variant-caps:small-caps",
        "line-height:40px",
        "text-wrap:balance",
        "text-wrap:nowrap",
        "white-space:pre",
        "white-space-collapse:preserve",
        "tab-size:2",
        "line-break:anywhere",
        "word-break:break-all",
        "overflow-wrap:anywhere",
        "hyphens:auto",
        "letter-spacing:2px",
        "word-spacing:3px",
        "text-transform:uppercase",
        "hanging-punctuation:first",
        "color:red",
        "text-shadow:1px 1px black",
        "text-decoration:underline",
        "visibility:hidden",
    ] {
        let css = format!("{BLOCK};{css}");
        assert!(!lifts_with_lines(COLUMNS, &css, "text"), "{css}");
        assert!(!lifts_with_lines(COLUMNS, &css, "inline"), "{css}");
    }
    // A no-break space renders, so it is a line whatever collapses.
    assert!(lifts_with_lines(COLUMNS, BLOCK, "nbsp"));
    assert!(!lifts_with_lines(
        COLUMNS,
        "display:block;font-size:30px",
        "nbsp"
    ));
    // Only white space or a comment between the blocks makes no line, when
    // the white space collapses.
    for css in [
        "text-align:center",
        "color:red",
        "text-decoration:underline",
        "font-size:30px",
    ] {
        let css = format!("{BLOCK};{css}");
        assert!(lifts_with_lines(COLUMNS, &css, "space"), "{css}");
        assert!(lifts_with_lines(COLUMNS, &css, "comment"), "{css}");
    }
}

#[test]
fn white_space_that_is_preserved_between_the_blocks_is_a_line() {
    for css in [
        "white-space:pre",
        "white-space:pre-wrap",
        "white-space:pre-line",
        "white-space:break-spaces",
        "white-space-collapse:preserve",
        "white-space-collapse:preserve-breaks",
        "white-space-collapse:preserve-spaces",
        "white-space-collapse:break-spaces",
    ] {
        // The wrapper preserves it, and the container does not.
        let wrapper = format!("{BLOCK};{css}");
        assert!(!lifts_with_lines(COLUMNS, &wrapper, "space"), "{wrapper}");
        // The container preserves it, and the wrapper does not.
        let container = format!("{COLUMNS};{css}");
        let wrapper = format!("{BLOCK};white-space:normal");
        assert!(
            !lifts_with_lines(&container, &wrapper, "space"),
            "{container}"
        );
        // Both preserve it, so the container's block lays it out the same.
        assert!(lifts_with_lines(&container, BLOCK, "space"), "{container}");
    }
    assert!(lifts_with_lines(
        COLUMNS,
        "display:block;white-space-collapse:discard",
        "space"
    ));
}

#[test]
fn a_spanner_outside_a_multicol_container_lifts_nothing() {
    for container_css in [
        "display:block;width:100px",
        // The block axis of a vertical container runs across the columns.
        "display:block;width:100px;columns:2;writing-mode:vertical-rl",
    ] {
        let (doc, wrapper) = wrapped_spanner(container_css, "div", "display:block");
        assert!(!doc.nodes[wrapper].generates_no_box(), "{container_css}");
    }
}

#[test]
fn a_box_that_is_not_a_spanner_lifts_nothing() {
    for css in [
        "display:block;column-span:all;height:10px;position:absolute",
        "display:block;column-span:all;height:10px;float:left",
        "display:none;column-span:all",
        "display:block;column-span:none;height:10px",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
        let container = doc.append_element(Some(body), "div", Style::default(), Some(COLUMNS));
        let wrapper = doc.append_element(Some(container), "div", Style::default(), Some(BLOCK));
        doc.append_element(Some(wrapper), "div", Style::default(), Some(css));
        lay_out(&mut doc);
        assert!(!doc.nodes[wrapper].generates_no_box(), "{css}");
    }
}

#[test]
fn a_spanner_with_a_logical_float_is_still_a_spanner() {
    // Only `left` and `right` float a box; the logical keywords do not.
    for float in ["inline-start", "inline-end"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
        let container = doc.append_element(Some(body), "div", Style::default(), Some(COLUMNS));
        let wrapper = doc.append_element(Some(container), "div", Style::default(), Some(BLOCK));
        let css = format!("display:block;column-span:all;height:10px;float:{float}");
        doc.append_element(Some(wrapper), "div", Style::default(), Some(css.as_str()));
        lay_out(&mut doc);
        assert!(doc.nodes[wrapper].generates_no_box(), "{css}");
    }
}

#[test]
fn laying_out_again_keeps_the_wrapper_lifted() {
    let (mut doc, wrapper) = wrapped_spanner(COLUMNS, "div", "display:block");
    lay_out(&mut doc);
    assert!(doc.nodes[wrapper].generates_no_box());
    let height = |doc: &Document| {
        doc.nodes[doc.nodes[wrapper].parent.unwrap()]
            .unrounded_layout
            .size
            .height
    };
    let first = height(&doc);
    lay_out(&mut doc);
    assert_eq!(height(&doc), first);
}

#[test]
fn lines_are_compared_with_the_container_not_with_the_parent() {
    // The lines of the inner wrapper go in an anonymous block of the
    // container, whatever its parent says, and so do the ones that a
    // `display: contents` child holds.
    for outer_css in [
        "display:block;text-align:center",
        "display:contents;text-align:center",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
        let container = doc.append_element(Some(body), "div", Style::default(), Some(COLUMNS));
        let outer = doc.append_element(Some(container), "div", Style::default(), Some(outer_css));
        let inner = doc.append_element(Some(outer), "div", Style::default(), Some(BLOCK));
        let contents = doc.append_element(
            Some(inner),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        doc.append_text(contents, "text");
        doc.append_element(
            Some(inner),
            "h2",
            Style::default(),
            Some("display:block;column-span:all;height:10px"),
        );
        lay_out(&mut doc);
        // The inner wrapper would leave its centered text to the container,
        // so it keeps its box, and the outer one cannot be lifted without it.
        assert!(!doc.nodes[inner].generates_no_box(), "{outer_css}");
        assert_eq!(
            doc.nodes[outer].generates_no_box(),
            outer_css.starts_with("display:contents"),
            "{outer_css}"
        );
    }
}

#[test]
fn a_wrapper_with_a_first_line_or_first_letter_style_keeps_its_box() {
    for (rule, lifted) in [
        ("div div::first-line{color:red}", false),
        ("div div::first-letter{color:red}", false),
        (
            "div div::first-line{font-size:2em}div div::first-letter{font-size:3em}",
            false,
        ),
        // The control: the same wrapper without such a style is lifted.
        ("div div::before{color:red}", true),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some(BLOCK));
        let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
        let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
        doc.append_text(style, rule);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
        let container = doc.append_element(Some(body), "div", Style::default(), Some(COLUMNS));
        let wrapper = doc.append_element(Some(container), "div", Style::default(), Some(BLOCK));
        doc.append_text(wrapper, "before");
        doc.append_element(
            Some(wrapper),
            "h2",
            Style::default(),
            Some("display:block;column-span:all;height:10px"),
        );
        doc.mark_in_document_flags();
        lay_out(&mut doc);
        assert_eq!(doc.nodes[wrapper].generates_no_box(), lifted, "{rule}");
    }
}
